import { describe, expect, it } from "vitest";

import { RecentSuccesses } from "./recentSuccesses";

describe("RecentSuccesses", () => {
  it("retains only the 64 newest unique success keys", () => {
    const successes = new RecentSuccesses();

    for (let index = 0; index <= 64; index += 1) {
      successes.remember(`operation-${index}`);
    }

    expect(successes.has("operation-0")).toBe(false);
    expect(successes.has("operation-1")).toBe(true);
    expect(successes.has("operation-64")).toBe(true);
  });

  it("does not evict history when the same success is remembered again", () => {
    const successes = new RecentSuccesses();

    for (let index = 0; index < 64; index += 1) {
      successes.remember(`operation-${index}`);
    }
    successes.remember("operation-0");

    expect(successes.has("operation-0")).toBe(true);
    expect(successes.has("operation-63")).toBe(true);
  });
});
