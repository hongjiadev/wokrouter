import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => {
  const render = vi.fn();
  const documentLocaleAtRootCreation: Array<{ lang: string; dir: string }> = [];
  return {
    createRoot: vi.fn(() => {
      documentLocaleAtRootCreation.push({
        lang: document.documentElement.lang,
        dir: document.documentElement.dir,
      });
      return { render };
    }),
    documentLocaleAtRootCreation,
    initializeI18n: vi.fn(),
    invoke: vi.fn(),
    render,
  };
});

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("react-dom/client", () => ({ createRoot: mocks.createRoot }));
vi.mock("./App", () => ({ App: () => null }));
vi.mock("./i18n", () => ({ initializeI18n: mocks.initializeI18n }));

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T | PromiseLike<T>) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: Deferred<T>["resolve"];
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

async function importMainWithPendingAutomaticBootstrap() {
  const automaticLocaleRequest = deferred<string>();
  mocks.invoke.mockReturnValueOnce(automaticLocaleRequest.promise);

  const main = await import("./main");

  expect(mocks.invoke).toHaveBeenCalledOnce();
  expect(mocks.initializeI18n).not.toHaveBeenCalled();
  expect(mocks.createRoot).not.toHaveBeenCalled();
  return main;
}

describe("desktop bootstrap", () => {
  beforeEach(() => {
    vi.resetModules();
    mocks.createRoot.mockClear();
    mocks.documentLocaleAtRootCreation.length = 0;
    mocks.initializeI18n.mockReset();
    mocks.invoke.mockReset();
    mocks.render.mockClear();
    document.documentElement.lang = "";
    document.documentElement.dir = "";
    document.body.innerHTML = '<div id="root"></div>';
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("waits for system locale and i18n before setting the document and rendering", async () => {
    const localeRequest = deferred<string>();
    const i18nInitialization = deferred<void>();
    mocks.invoke.mockReturnValueOnce(localeRequest.promise);
    mocks.initializeI18n.mockReturnValueOnce(i18nInitialization.promise);

    await import("./main");

    expect(mocks.invoke).toHaveBeenCalledWith("system_locale");
    expect(mocks.initializeI18n).not.toHaveBeenCalled();
    expect(mocks.createRoot).not.toHaveBeenCalled();

    localeRequest.resolve("zh-CN");
    await vi.waitFor(() => {
      expect(mocks.initializeI18n).toHaveBeenCalledWith("zh-CN");
    });
    expect(document.documentElement.lang).toBe("");
    expect(document.documentElement.dir).toBe("");
    expect(mocks.createRoot).not.toHaveBeenCalled();

    i18nInitialization.resolve();
    await vi.waitFor(() => {
      expect(mocks.createRoot).toHaveBeenCalledOnce();
    });

    expect(document.documentElement.lang).toBe("zh-CN");
    expect(document.documentElement.dir).toBe("ltr");
    expect(mocks.documentLocaleAtRootCreation).toEqual([
      { lang: "zh-CN", dir: "ltr" },
    ]);
    expect(mocks.render).toHaveBeenCalledOnce();
  });

  it("falls back to navigator candidates when the system locale invoke fails", async () => {
    vi.spyOn(window.navigator, "languages", "get").mockReturnValue([
      "zh-Hans",
      "en-US",
    ]);
    vi.spyOn(window.navigator, "language", "get").mockReturnValue("en-US");
    mocks.invoke.mockRejectedValueOnce(new Error("bridge unavailable"));
    mocks.initializeI18n.mockResolvedValueOnce(undefined);

    await import("./main");

    await vi.waitFor(() => {
      expect(mocks.createRoot).toHaveBeenCalledOnce();
    });
    expect(mocks.initializeI18n).toHaveBeenCalledWith("zh-CN");
    expect(document.documentElement.lang).toBe("zh-CN");
    expect(document.documentElement.dir).toBe("ltr");
    expect(mocks.render).toHaveBeenCalledOnce();
  });

  it("rejects and never renders when i18n initialization fails", async () => {
    const initializationError = new Error("catalog initialization failed");
    const { bootstrap } = await importMainWithPendingAutomaticBootstrap();
    document.documentElement.lang = "existing-locale";
    document.documentElement.dir = "rtl";
    mocks.invoke.mockResolvedValueOnce("zh-CN");
    mocks.initializeI18n.mockRejectedValueOnce(initializationError);

    await expect(bootstrap()).rejects.toBe(initializationError);

    expect(mocks.invoke).toHaveBeenCalledTimes(2);
    expect(mocks.initializeI18n).toHaveBeenCalledOnce();
    expect(mocks.createRoot).not.toHaveBeenCalled();
    expect(mocks.render).not.toHaveBeenCalled();
    expect(document.documentElement.lang).toBe("existing-locale");
    expect(document.documentElement.dir).toBe("rtl");
  });

  it("rejects the exact missing-root error without rendering elsewhere", async () => {
    const { bootstrap } = await importMainWithPendingAutomaticBootstrap();
    document.body.innerHTML = "";
    mocks.invoke.mockResolvedValueOnce("en-US");
    mocks.initializeI18n.mockResolvedValueOnce(undefined);

    await expect(bootstrap()).rejects.toThrowError(
      /^WokRouter desktop root is missing\.$/,
    );

    expect(mocks.invoke).toHaveBeenCalledTimes(2);
    expect(mocks.initializeI18n).toHaveBeenCalledWith("en");
    expect(mocks.createRoot).not.toHaveBeenCalled();
    expect(mocks.render).not.toHaveBeenCalled();
  });
});
