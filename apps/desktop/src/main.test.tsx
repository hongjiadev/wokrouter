import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => {
  const render = vi.fn();
  return {
    createRoot: vi.fn(() => ({ render })),
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

describe("desktop bootstrap", () => {
  beforeEach(() => {
    vi.resetModules();
    mocks.createRoot.mockClear();
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
});
