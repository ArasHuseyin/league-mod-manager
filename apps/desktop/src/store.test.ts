import { describe, expect, it } from "vitest";

describe("desktop scaffold", () => {
  it("keeps a smoke test target for CI wiring", () => {
    expect("league-mod-manager").toContain("manager");
  });
});
