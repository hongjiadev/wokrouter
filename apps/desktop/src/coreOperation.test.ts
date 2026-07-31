import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  checkCoreUpdate,
  getCoreOperation,
  installAndStartCore,
  installCoreUpdate,
  listenForCoreOperation,
  parseCoreOperation,
  parseCoreUpdateCheck,
  type CoreOperation,
} from "./coreOperation";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

const INSTALL_ID = "64c09bda-7afd-4e86-8d61-43bc39a8bc51";
const RETRY_ID = "9eb267f2-4ef0-48f6-b3c6-c916d1a7ab8e";

function operation(
  fields: Partial<Record<string, unknown>> = {},
): Record<string, unknown> {
  return {
    schema_version: 1,
    operation_id: INSTALL_ID,
    sequence: 0,
    operation: "install",
    state: "running",
    phase: "checking_release",
    ...fields,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

type EventHandler = (event: { payload: unknown }) => void;

function mockEventBus() {
  const handlers = new Set<EventHandler>();
  const rawUnlistens: ReturnType<typeof vi.fn>[] = [];
  vi.mocked(listen).mockImplementation(async (_event, callback) => {
    const handler = callback as EventHandler;
    handlers.add(handler);
    const rawUnlisten = vi.fn(() => {
      handlers.delete(handler);
    });
    rawUnlistens.push(rawUnlisten);
    return rawUnlisten;
  });
  return {
    emit(payload: unknown) {
      for (const handler of [...handlers]) {
        handler({ payload });
      }
    },
    rawUnlistens,
    listenerCount: () => handlers.size,
  };
}

describe("core operation schemas", () => {
  it("copies only recognized fields from a same-schema determinate download", () => {
    expect(
      parseCoreOperation(
        operation({
          sequence: 3,
          phase: "downloading",
          target_version: "0.1.23",
          bytes_completed: 512,
          bytes_total: 1024,
          future_optional: { private_path: "C:\\must-not-cross.ts" },
        }),
      ),
    ).toEqual({
      schemaVersion: 1,
      operationId: INSTALL_ID,
      sequence: 3,
      operation: "install",
      state: "running",
      phase: "downloading",
      targetVersion: "0.1.23",
      bytesCompleted: 512,
      bytesTotal: 1024,
    });
  });

  it("accepts an indeterminate phase without byte fields", () => {
    expect(
      parseCoreOperation(
        operation({
          operation: "update",
          phase: "verifying",
          current_version: "0.1.22",
          target_version: "0.1.23-beta.1+signed",
        }),
      ),
    ).toMatchObject({
      operation: "update",
      phase: "verifying",
      currentVersion: "0.1.22",
      targetVersion: "0.1.23-beta.1+signed",
    });
  });

  it.each([
    ["unknown schema", { schema_version: 2 }],
    ["invalid UUID", { operation_id: "not-an-operation-id" }],
    ["negative sequence", { sequence: -1 }],
    ["fractional sequence", { sequence: 0.5 }],
    ["unsafe sequence", { sequence: Number.MAX_SAFE_INTEGER + 1 }],
    ["non-canonical semver", { target_version: "v0.1.23" }],
    ["download without byte fields", { phase: "downloading" }],
    [
      "zero download total",
      { phase: "downloading", bytes_completed: 0, bytes_total: 0 },
    ],
    [
      "download beyond total",
      { phase: "downloading", bytes_completed: 2, bytes_total: 1 },
    ],
    ["bytes outside download", { bytes_completed: 0, bytes_total: 1 }],
    ["install-only operation phase", { operation: "update", phase: "authorizing" }],
    ["update-only operation phase", { phase: "draining" }],
    ["running completed state", { phase: "completed" }],
    [
      "running state with error",
      { error_code: "download_failed" },
    ],
    ["succeeded before completed", { state: "succeeded", phase: "installing" }],
    [
      "succeeded with error",
      {
        state: "succeeded",
        phase: "completed",
        error_code: "download_failed",
      },
    ],
    ["failed without error", { state: "failed", phase: "completed" }],
    [
      "unknown stable error",
      { state: "failed", phase: "completed", error_code: "C:\\secret" },
    ],
    [
      "error for the wrong operation",
      {
        state: "failed",
        phase: "completed",
        error_code: "update_install_failed",
      },
    ],
    ["active requests on install", { active_requests: 1 }],
    [
      "active request cap exceeded",
      {
        operation: "update",
        phase: "draining",
        active_requests: 1_000_001,
      },
    ],
  ])("rejects %s", (_case, fields) => {
    expect(() => parseCoreOperation(operation(fields))).toThrow(
      "Invalid WokCore operation",
    );
  });

  it("accepts only strict recognized update-check fields", () => {
    expect(
      parseCoreUpdateCheck({
        code: "update_available",
        current_version: "0.1.22",
        target_version: "0.1.23",
        future_optional: "ignored",
      }),
    ).toEqual({
      code: "update_available",
      currentVersion: "0.1.22",
      targetVersion: "0.1.23",
    });
    expect(() =>
      parseCoreUpdateCheck({
        code: "current",
        current_version: "0.1.22",
        target_version: "0.1.23",
      }),
    ).toThrow("Invalid WokCore update check");
  });
});

describe("core operation bridge", () => {
  const subscriptions: UnlistenFn[] = [];

  beforeEach(async () => {
    vi.mocked(invoke).mockReset();
    vi.mocked(listen).mockReset();
    vi.mocked(invoke).mockResolvedValueOnce(null);
    await getCoreOperation();
  });

  afterEach(() => {
    for (const unlisten of subscriptions.splice(0)) {
      unlisten();
    }
  });

  async function subscribe(
    listener: (snapshot: CoreOperation) => void,
  ): Promise<UnlistenFn> {
    const unlisten = await listenForCoreOperation(listener);
    subscriptions.push(unlisten);
    return unlisten;
  }

  it("invokes the narrow commands and parses every unknown response", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(operation())
      .mockResolvedValueOnce({
        code: "current",
        current_version: "0.1.22",
      })
      .mockResolvedValueOnce(
        operation({
          operation_id: RETRY_ID,
          operation: "update",
        }),
      );

    await expect(installAndStartCore()).resolves.toMatchObject({
      operationId: INSTALL_ID,
    });
    await expect(checkCoreUpdate()).resolves.toEqual({
      code: "current",
      currentVersion: "0.1.22",
    });
    await expect(installCoreUpdate("0.1.23")).resolves.toMatchObject({
      operationId: RETRY_ID,
      operation: "update",
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "install_and_start_core");
    expect(invoke).toHaveBeenNthCalledWith(3, "check_core_update");
    expect(invoke).toHaveBeenNthCalledWith(4, "install_core_update", {
      expectedVersion: "0.1.23",
    });
  });

  it("rejects malformed command responses", async () => {
    vi.mocked(invoke).mockResolvedValueOnce({
      schema_version: 1,
      raw_error: "C:\\private\\wokcore.exe",
    });

    await expect(installAndStartCore()).rejects.toThrow(
      "Invalid WokCore operation",
    );
  });

  it("delivers a terminal status authority exactly once before its matching event", async () => {
    const bus = mockEventBus();
    const delivered: CoreOperation[] = [];
    await subscribe((snapshot) => delivered.push(snapshot));
    bus.emit(operation({ sequence: 1, phase: "starting" }));
    const terminal = operation({
      sequence: 2,
      state: "succeeded",
      phase: "completed",
    });
    vi.mocked(invoke).mockResolvedValueOnce(terminal);

    await expect(getCoreOperation()).resolves.toMatchObject({
      operationId: INSTALL_ID,
      sequence: 2,
      state: "succeeded",
    });
    bus.emit(terminal);

    expect(delivered.map(({ sequence, state }) => [sequence, state])).toEqual([
      [1, "running"],
      [2, "succeeded"],
    ]);
  });

  it("delivers one authoritative terminal snapshot to every listener", async () => {
    const bus = mockEventBus();
    const first: CoreOperation[] = [];
    const second: CoreOperation[] = [];
    await subscribe((snapshot) => first.push(snapshot));
    await subscribe((snapshot) => second.push(snapshot));
    bus.emit(operation({ sequence: 1, phase: "starting" }));
    const terminal = operation({
      sequence: 2,
      state: "failed",
      phase: "completed",
      error_code: "start_failed",
    });
    vi.mocked(invoke).mockResolvedValueOnce(terminal);

    await getCoreOperation();
    bus.emit(terminal);

    expect(first.map(({ sequence }) => sequence)).toEqual([1, 2]);
    expect(second.map(({ sequence }) => sequence)).toEqual([1, 2]);
  });

  it("returns the newer event authority when an older invoke resolves late", async () => {
    const oldInvoke = deferred<unknown>();
    vi.mocked(invoke).mockReturnValueOnce(oldInvoke.promise);
    const pendingInvoke = installAndStartCore();
    const bus = mockEventBus();
    const delivered: CoreOperation[] = [];
    await subscribe((snapshot) => delivered.push(snapshot));
    bus.emit(
      operation({
        operation_id: RETRY_ID,
        sequence: 2,
        phase: "starting",
      }),
    );

    oldInvoke.resolve(operation({ sequence: 0 }));

    await expect(pendingInvoke).resolves.toMatchObject({
      operationId: RETRY_ID,
      sequence: 2,
    });
    expect(delivered).toHaveLength(1);
  });

  it("keeps an equal-sequence event authoritative over a delayed null status", async () => {
    const matchingInvoke = deferred<unknown>();
    const staleStatus = deferred<unknown>();
    vi.mocked(invoke)
      .mockReturnValueOnce(matchingInvoke.promise)
      .mockReturnValueOnce(staleStatus.promise);
    const pendingInvoke = installAndStartCore();
    const pendingStatus = getCoreOperation();
    const bus = mockEventBus();
    const delivered: CoreOperation[] = [];
    await subscribe((snapshot) => delivered.push(snapshot));
    bus.emit(operation());

    matchingInvoke.resolve(operation());
    await expect(pendingInvoke).resolves.toMatchObject({
      operationId: INSTALL_ID,
      sequence: 0,
    });
    staleStatus.resolve(null);
    await expect(pendingStatus).resolves.toMatchObject({
      operationId: INSTALL_ID,
      sequence: 0,
    });

    expect(delivered.map(({ operationId, sequence }) => [operationId, sequence]))
      .toEqual([[INSTALL_ID, 0]]);
    const remounted: CoreOperation[] = [];
    await subscribe((snapshot) => remounted.push(snapshot));
    expect(remounted).toEqual([]);
    bus.emit(operation({ sequence: 1, phase: "starting" }));
    expect(remounted.map(({ sequence }) => sequence)).toEqual([1]);
  });

  it("keeps an equal-sequence event authoritative over an older UUID status", async () => {
    const matchingStatus = deferred<unknown>();
    const staleStatus = deferred<unknown>();
    vi.mocked(invoke)
      .mockReturnValueOnce(matchingStatus.promise)
      .mockReturnValueOnce(staleStatus.promise);
    const pendingMatching = getCoreOperation();
    const pendingStale = getCoreOperation();
    const bus = mockEventBus();
    const delivered: CoreOperation[] = [];
    await subscribe((snapshot) => delivered.push(snapshot));
    bus.emit(operation());

    matchingStatus.resolve(operation());
    await expect(pendingMatching).resolves.toMatchObject({
      operationId: INSTALL_ID,
      sequence: 0,
    });
    staleStatus.resolve(
      operation({
        operation_id: RETRY_ID,
        sequence: 9,
        phase: "starting",
      }),
    );
    await expect(pendingStale).resolves.toMatchObject({
      operationId: INSTALL_ID,
      sequence: 0,
    });
    expect(delivered.map(({ operationId, sequence }) => [operationId, sequence]))
      .toEqual([[INSTALL_ID, 0]]);
  });

  it("does not compare sequence values across authoritative UUID changes", async () => {
    const bus = mockEventBus();
    const delivered: CoreOperation[] = [];
    await subscribe((snapshot) => delivered.push(snapshot));
    bus.emit(operation({ sequence: 9, phase: "starting" }));
    vi.mocked(invoke).mockResolvedValueOnce(
      operation({
        operation_id: RETRY_ID,
        sequence: 0,
      }),
    );

    await expect(installAndStartCore()).resolves.toMatchObject({
      operationId: RETRY_ID,
      sequence: 0,
    });
    expect(delivered.map(({ operationId, sequence }) => [operationId, sequence]))
      .toEqual([
        [INSTALL_ID, 9],
        [RETRY_ID, 0],
      ]);
  });

  it("keeps subscriptions independent and makes unlisten idempotent", async () => {
    const bus = mockEventBus();
    const first: CoreOperation[] = [];
    const second: CoreOperation[] = [];
    const stopFirst = await subscribe((snapshot) => first.push(snapshot));
    const stopSecond = await subscribe((snapshot) => second.push(snapshot));
    bus.emit(operation());

    stopFirst();
    stopFirst();
    expect(bus.rawUnlistens[0]).toHaveBeenCalledOnce();
    expect(bus.listenerCount()).toBe(1);
    bus.emit(operation({ sequence: 1, phase: "starting" }));

    expect(first.map(({ sequence }) => sequence)).toEqual([0]);
    expect(second.map(({ sequence }) => sequence)).toEqual([0, 1]);

    stopSecond();
    stopSecond();
    expect(bus.rawUnlistens[1]).toHaveBeenCalledOnce();
    expect(bus.listenerCount()).toBe(0);
    vi.mocked(invoke).mockResolvedValueOnce(
      operation({
        sequence: 2,
        state: "succeeded",
        phase: "completed",
      }),
    );
    await getCoreOperation();
    expect(first.map(({ sequence }) => sequence)).toEqual([0]);
    expect(second.map(({ sequence }) => sequence)).toEqual([0, 1]);

    const remounted: CoreOperation[] = [];
    await subscribe((snapshot) => remounted.push(snapshot));
    expect(remounted).toEqual([]);
  });

  it("ignores stale sequence and invalid cross-event byte progress", async () => {
    const bus = mockEventBus();
    const delivered: CoreOperation[] = [];
    const stop = await subscribe((snapshot) => {
      delivered.push(snapshot);
    });

    bus.emit(
      operation({
        sequence: 3,
        phase: "downloading",
        bytes_completed: 512,
        bytes_total: 1024,
      }),
    );
    bus.emit(
      operation({
        sequence: 3,
        phase: "downloading",
        bytes_completed: 600,
        bytes_total: 1024,
      }),
    );
    bus.emit(
      operation({
        sequence: 4,
        phase: "downloading",
        bytes_completed: 511,
        bytes_total: 1024,
      }),
    );
    bus.emit(
      operation({
        sequence: 4,
        phase: "downloading",
        bytes_completed: 600,
        bytes_total: 2048,
      }),
    );
    bus.emit(
      operation({
        sequence: 4,
        phase: "downloading",
        bytes_completed: 768,
        bytes_total: 1024,
      }),
    );

    expect(delivered.map(({ sequence }) => sequence)).toEqual([3, 4]);
    expect(delivered.at(-1)).toMatchObject({
      bytesCompleted: 768,
      bytesTotal: 1024,
    });
    stop();
    expect(bus.rawUnlistens[0]).toHaveBeenCalledOnce();
  });

  it("allows a new UUID after terminal and never lets the retired UUID overwrite it", async () => {
    const bus = mockEventBus();
    const delivered: CoreOperation[] = [];
    await subscribe((snapshot) => delivered.push(snapshot));

    bus.emit(
      operation({
        sequence: 8,
        state: "failed",
        phase: "completed",
        error_code: "start_failed",
      }),
    );
    bus.emit(
      operation({
        operation_id: RETRY_ID,
        sequence: 0,
      }),
    );
    bus.emit(
      operation({
        sequence: 9,
        state: "failed",
        phase: "completed",
        error_code: "start_failed",
      }),
    );
    bus.emit(
      operation({
        operation_id: RETRY_ID,
        sequence: 1,
        phase: "starting",
      }),
    );

    expect(delivered.map(({ operationId, sequence }) => [operationId, sequence]))
      .toEqual([
        [INSTALL_ID, 8],
        [RETRY_ID, 0],
        [RETRY_ID, 1],
      ]);
  });

  it("does not revive an old UUID after the retry reaches terminal state", async () => {
    const bus = mockEventBus();
    const delivered: CoreOperation[] = [];
    await subscribe((snapshot) => delivered.push(snapshot));

    bus.emit(
      operation({
        sequence: 4,
        state: "failed",
        phase: "completed",
        error_code: "start_failed",
      }),
    );
    bus.emit(operation({ operation_id: RETRY_ID, sequence: 0 }));
    bus.emit(
      operation({
        operation_id: RETRY_ID,
        sequence: 1,
        state: "succeeded",
        phase: "completed",
      }),
    );
    bus.emit(operation({ sequence: 3, phase: "starting" }));

    expect(delivered.map(({ operationId, sequence }) => [operationId, sequence]))
      .toEqual([
        [INSTALL_ID, 4],
        [RETRY_ID, 0],
        [RETRY_ID, 1],
      ]);
  });

  it("uses an invoke result as authority when a retry event races the response", async () => {
    const bus = mockEventBus();
    const delivered: CoreOperation[] = [];
    await subscribe((snapshot) => delivered.push(snapshot));
    bus.emit(operation({ sequence: 4, phase: "starting" }));

    const retry = deferred<unknown>();
    vi.mocked(invoke).mockReturnValueOnce(retry.promise);
    const accepted = installAndStartCore();
    bus.emit(
      operation({
        operation_id: RETRY_ID,
        sequence: 1,
        phase: "starting",
      }),
    );
    retry.resolve(operation({ operation_id: RETRY_ID, sequence: 0 }));
    await expect(accepted).resolves.toMatchObject({
      operationId: RETRY_ID,
      sequence: 0,
    });
    bus.emit(
      operation({
        sequence: 5,
        state: "failed",
        phase: "completed",
        error_code: "start_failed",
      }),
    );
    bus.emit(
      operation({
        operation_id: RETRY_ID,
        sequence: 2,
        phase: "authorizing",
      }),
    );

    expect(delivered.map(({ operationId, sequence }) => [operationId, sequence]))
      .toEqual([
        [INSTALL_ID, 4],
        [RETRY_ID, 0],
        [RETRY_ID, 2],
      ]);
  });

  it("does not let an older status request override a newer operation event", async () => {
    const oldStatus = deferred<unknown>();
    vi.mocked(invoke).mockReturnValueOnce(oldStatus.promise);
    const pendingStatus = getCoreOperation();

    const bus = mockEventBus();
    const delivered: CoreOperation[] = [];
    await subscribe((snapshot) => delivered.push(snapshot));
    bus.emit(
      operation({
        operation_id: RETRY_ID,
        sequence: 2,
        phase: "starting",
      }),
    );
    oldStatus.resolve(
      operation({
        sequence: 7,
        phase: "starting",
      }),
    );
    await expect(pendingStatus).resolves.toMatchObject({
      operationId: RETRY_ID,
      sequence: 2,
    });
    bus.emit(
      operation({
        sequence: 8,
        state: "failed",
        phase: "completed",
        error_code: "start_failed",
      }),
    );
    const remounted: CoreOperation[] = [];
    await subscribe((snapshot) => remounted.push(snapshot));
    bus.emit(
      operation({
        operation_id: RETRY_ID,
        sequence: 3,
        phase: "authorizing",
      }),
    );

    expect(delivered.map(({ operationId, sequence }) => [operationId, sequence]))
      .toEqual([
        [RETRY_ID, 2],
        [RETRY_ID, 3],
      ]);
    expect(remounted.map(({ operationId, sequence }) => [operationId, sequence]))
      .toEqual([[RETRY_ID, 3]]);
  });
});
