import { describe, expect, it } from "vitest";
import { clearIfUnchanged, type ClearEnv, type ReadAccess } from "./clipboard";

const PHRASE = "abandon ability able about above absent absorb abstract";

function env(opts: {
  content: string;
  focus?: boolean;
  access?: ReadAccess;
  readThrows?: boolean;
  accessThrows?: boolean;
}): ClearEnv & { content: () => string } {
  let content = opts.content;
  return {
    content: () => content,
    clipboard: {
      writeText: (t) => {
        content = t;
        return Promise.resolve();
      },
      readText: () =>
        opts.readThrows === true
          ? Promise.reject(new DOMException("denied", "NotAllowedError"))
          : Promise.resolve(content),
    },
    hasFocus: () => opts.focus ?? true,
    readAccess: () =>
      opts.accessThrows === true
        ? Promise.reject(new TypeError("unsupported"))
        : Promise.resolve(opts.access ?? "granted"),
  };
}

describe("clearing the copied phrase", () => {
  it("clears it when it is still there and the page has focus", async () => {
    const e = env({ content: PHRASE });
    expect(await clearIfUnchanged(e, PHRASE)).toBe("cleared");
    expect(e.content()).toBe("");
  });

  it("leaves something else the person copied since", async () => {
    const e = env({ content: "my shopping list" });
    expect(await clearIfUnchanged(e, PHRASE)).toBe("changed");
    expect(e.content()).toBe("my shopping list");
  });

  it("does nothing without focus", async () => {
    const e = env({ content: PHRASE, focus: false });
    expect(await clearIfUnchanged(e, PHRASE)).toBe("skipped");
    expect(e.content()).toBe(PHRASE);
  });

  it("never asks for a permission (Chromium without clipboard-read)", async () => {
    const e = env({ content: PHRASE, access: "would-prompt" });
    expect(await clearIfUnchanged(e, PHRASE)).toBe("skipped");
    expect(e.content()).toBe(PHRASE);
  });

  it("tries a same-origin read where there is no permission API (Firefox, Safari)", async () => {
    const e = env({ content: PHRASE, access: "no-permission-api" });
    expect(await clearIfUnchanged(e, PHRASE)).toBe("cleared");
  });

  it("never throws when reading is refused", async () => {
    const e = env({ content: PHRASE, readThrows: true });
    expect(await clearIfUnchanged(e, PHRASE)).toBe("skipped");
    const f = env({ content: PHRASE, accessThrows: true });
    expect(await clearIfUnchanged(f, PHRASE)).toBe("skipped");
  });
});
