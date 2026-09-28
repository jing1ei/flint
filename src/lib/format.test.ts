import { describe, expect, it } from "vitest";
import { parentDir } from "./format";

describe("output folders", () => {
  it.each([
    ["C:\\clip.mp4", "C:\\"],
    ["C:/clip.mp4", "C:/"],
    ["C:\\Converted\\clip.mp4", "C:\\Converted"],
    ["\\\\server\\share\\clip.mp4", "\\\\server\\share"],
    ["/clip.mp4", "/"],
    ["/Converted/clip.mp4", "/Converted"],
    ["clip.mp4", ""],
  ])("finds the absolute folder of %s", (path, expected) => {
    expect(parentDir(path)).toBe(expected);
  });
});
