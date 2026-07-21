import { describe, expect, it } from "vitest";
import { applyTransforms, migrateLegacyTransform, serializeTransform, type MappingTransform } from "./mapping-transforms";

describe("mapping transformations", () => {
  it("executes an ordered transformation pipeline for the browser preview", () => {
    const transforms: MappingTransform[] = [
      { kind: "trim" },
      { kind: "lowercase" },
      { kind: "replace", from: " ", to: "-" },
    ];
    expect(applyTransforms("  Hello World  ", transforms, {})).toBe("hello-world");
  });

  it("uses source fields for coalesce and concatenation", () => {
    expect(applyTransforms(null, [
      { kind: "coalesce", fields: ["fallback"] },
      { kind: "concat", fields: ["suffix"], separator: " / " },
    ], { fallback: "Primary", suffix: "Secondary" })).toBe("Primary / Secondary");
  });

  it("migrates legacy transforms and serializes typed defaults", () => {
    expect(migrateLegacyTransform("Trim")).toEqual([{ kind: "trim" }]);
    expect(serializeTransform({ kind: "default", value: "42" })).toEqual({ kind: "default", value: 42 });
    expect(serializeTransform({ kind: "default", value: "unknown" })).toEqual({ kind: "default", value: "unknown" });
  });
});
