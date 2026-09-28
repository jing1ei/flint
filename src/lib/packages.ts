/**
 * The join between what a user installs and what this machine has.
 *
 * `get_install_plans` says nothing about presence and `refresh_tools` says nothing about packages:
 * one is a list of things to buy, the other a list of binaries on a disk. Everything the UI shows
 * about a helper — its state word, whether the Install button is there, whether a failed row can
 * point at it — comes from putting the two together here, so there is exactly one derivation of it
 * and `convert_core::package::Presence` has exactly one counterpart on this side.
 */
import type { PackageInstallPlan, ToolStatus } from "./types";

/**
 * How much of a package is here — `convert_core::package::Presence`.
 *
 * `"incomplete"` is real and has to be said out loud: `brew install poppler` normally lands all
 * three binaries, but a pruned Cellar or a half-finished install can leave one behind, and the
 * conversion that needs *that* binary still cannot run. Such a package is **not installed** (it may
 * not claim to be) and **still installable** (one more click is the fix).
 */
export type Presence = "installed" | "incomplete" | "absent";

/** The member binaries of `plan`, in plan order, as the tool list knows them. */
export function membersOf(plan: PackageInstallPlan, tools: ToolStatus[]): ToolStatus[] {
  return plan.tool_ids
    .map((id) => tools.find((tool) => tool.id === id))
    .filter((tool): tool is ToolStatus => tool !== undefined);
}

/**
 * Presence, derived by joining `plan.tool_ids` against `ToolStatus.id`: all present → installed,
 * some → incomplete, none → absent.
 *
 * A plan whose members are not in the tool list at all (an older shell answering a newer plan) is
 * `"absent"` rather than silently complete: an empty `every` is true, and "installed" is the one
 * answer that must never be guessed.
 */
export function presenceOf(plan: PackageInstallPlan, tools: ToolStatus[]): Presence {
  const members = membersOf(plan, tools);
  if (members.length === 0) return "absent";
  const found = members.filter((tool) => tool.available).length;
  if (found === members.length) return "installed";
  return found === 0 ? "absent" : "incomplete";
}

/** Members this machine is missing. Diagnostic only — `ToolStatus.label` is not install copy. */
export const missingMembers = (plan: PackageInstallPlan, tools: ToolStatus[]): ToolStatus[] =>
  membersOf(plan, tools).filter((tool) => !tool.available);

/** How many of the package's programs are here, and how many there are: "1 of 3". */
export function memberCount(
  plan: PackageInstallPlan,
  tools: ToolStatus[],
): { found: number; total: number } {
  const members = membersOf(plan, tools);
  return { found: members.filter((tool) => tool.available).length, total: members.length };
}

/**
 * The plan a *name* refers to. `FormatView.needs` and a file's note are package names ("Poppler"),
 * because that is what a person installs — this turns one back into the row it came from.
 */
export const planNamed = (
  plans: PackageInstallPlan[],
  name: string,
): PackageInstallPlan | undefined => plans.find((plan) => plan.name === name);
