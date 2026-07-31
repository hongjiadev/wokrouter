import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { beforeEach, describe, expect, it, vi } from "vitest";

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
  beforeEach(async () => {
    vi.mocked(invoke).mockReset();
    vi.mocked(listen).mockReset();
    vi.mocked(invoke).mockResolvedValueOnce(null);
    await getCoreOperation();
  });

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

  it("ignores stale sequence and invalid cross-event byte progress", async () => {
    let handler: EventHandler | undefined;
    const unlisten = vi.fn();
    vi.mocked(listen).mockImplementation(async (_event, callback) => {
      handler = callback as EventHandler;
      return unlisten;
    });
    const delivered: CoreOperation[] = [];
    const stop = await listenForCoreOperation((snapshot) => {
      delivered.push(snapshot);
    });

    handler?.({
      payload: operation({
        sequence: 3,
        phase: "downloading",
        bytes_completed: 512,
        bytes_total: 1024,
      }),
    });
    handler?.({
      payload: operation({
        sequence: 3,
        phase: "downloading",
        bytes_completed: 600,
        bytes_total: 1024,
      }),
    });
    handler?.({
      payload: operation({
        sequence: 4,
        phase: "downloading",
        bytes_completed: 511,
        bytes_total: 1024,
      }),
    });
    handler?.({
      payload: operation({
        sequence: 4,
        phase: "downloading",
        bytes_completed: 600,
        bytes_total: 2048,
      }),
    });
    handler?.({
      payload: operation({
        sequence: 4,
        phase: "downloading",
        bytes_completed: 768,
        bytes_total: 1024,
      }),
    });

    expect(delivered.map(({ sequence }) => sequence)).toEqual([3, 4]);
    expect(delivered.at(-1)).toMatchObject({
      bytesCompleted: 768,
      bytesTotal: 1024,
    });
    stop();
    expect(unlisten).toHaveBeenCalledOnce();
  });

  it("allows a new UUID after terminal and never lets the retired UUID overwrite it", async () => {
    let handler: EventHandler | undefined;
    vi.mocked(listen).mockImplementation(async (_event, callback) => {
      handler = callback as EventHandler;
      return () => undefined;
    });
    const delivered: CoreOperation[] = [];
    await listenForCoreOperation((snapshot) => delivered.push(snapshot));

    handler?.({
      payload: operation({
        sequence: 8,
        state: "failed",
        phase: "completed",
        error_code: "start_failed",
      }),
    });
    handler?.({
      payload: operation({
        operation_id: RETRY_ID,
        sequence: 0,
      }),
    });
    handler?.({
      payload: operation({
        sequence: 9,
        state: "failed",
        phase: "completed",
        error_code: "start_failed",
      }),
    });
    handler?.({
      payload: operation({
        operation_id: RETRY_ID,
        sequence: 1,
        phase: "starting",
      }),
    });

    expect(delivered.map(({ operationId, sequence }) => [operationId, sequence]))
      .toEqual([
        [INSTALL_ID, 8],
        [RETRY_ID, 0],
        [RETRY_ID, 1],
      ]);
  });

  it("does not revive an old UUID after the retry reaches terminal state", async () => {
    let handler: EventHandler | undefined;
    vi.mocked(listen).mockImplementation(async (_event, callback) => {
      handler = callback as EventHandler;
      return () => undefined;
    });
    const delivered: CoreOperation[] = [];
    await listenForCoreOperation((snapshot) => delivered.push(snapshot));

    handler?.({
      payload: operation({
        sequence: 4,
        state: "failed",
        phase: "completed",
        error_code: "start_failed",
      }),
    });
    handler?.({
      payload: operation({ operation_id: RETRY_ID, sequence: 0 }),
    });
    handler?.({
      payload: operation({
        operation_id: RETRY_ID,
        sequence: 1,
        state: "succeeded",
        phase: "completed",
      }),
    });
    handler?.({
      payload: operation({ sequence: 3, phase: "starting" }),
    });

    expect(delivered.map(({ operationId, sequence }) => [operationId, sequence]))
      .toEqual([
        [INSTALL_ID, 4],
        [RETRY_ID, 0],
        [RETRY_ID, 1],
      ]);
  });

  it("uses an invoke result as authority when a retry event races the response", async () => {
    let handler: EventHandler | undefined;
    vi.mocked(listen).mockImplementation(async (_event, callback) => {
      handler = callback as EventHandler;
      return () => undefined;
    });
    const delivered: CoreOperation[] = [];
    await listenForCoreOperation((snapshot) => delivered.push(snapshot));
    handler?.({
      payload: operation({ sequence: 4, phase: "starting" }),
    });

    const retry = deferred<unknown>();
    vi.mocked(invoke).mockReturnValueOnce(retry.promise);
    const accepted = installAndStartCore();
    handler?.({
      payload: operation({
        operation_id: RETRY_ID,
        sequence: 1,
        phase: "starting",
      }),
    });
    retry.resolve(operation({ operation_id: RETRY_ID, sequence: 0 }));
    await expect(accepted).resolves.toMatchObject({
      operationId: RETRY_ID,
      sequence: 0,
    });
    handler?.({
      payload: operation({
        sequence: 5,
        state: "failed",
        phase: "completed",
        error_code: "start_failed",
      }),
    });
    handler?.({
      payload: operation({
        operation_id: RETRY_ID,
        sequence: 2,
        phase: "authorizing",
      }),
    });

    expect(delivered.map(({ operationId, sequence }) => [operationId, sequence]))
      .toEqual([
        [INSTALL_ID, 4],
        [RETRY_ID, 2],
      ]);
  });

  it("does not let an older status request override a newer operation event", async () => {
    const oldStatus = deferred<unknown>();
    vi.mocked(invoke).mockReturnValueOnce(oldStatus.promise);
    const pendingStatus = getCoreOperation();

    let handler: EventHandler | undefined;
    vi.mocked(listen).mockImplementation(async (_event, callback) => {
      handler = callback as EventHandler;
      return () => undefined;
    });
    const delivered: CoreOperation[] = [];
    await listenForCoreOperation((snapshot) => delivered.push(snapshot));
    handler?.({
      payload: operation({
        operation_id: RETRY_ID,
        sequence: 2,
        phase: "starting",
      }),
    });
    oldStatus.resolve(
      operation({
        sequence: 7,
        phase: "starting",
      }),
    );
    await pendingStatus;
    handler?.({
      payload: operation({
        sequence: 8,
        state: "failed",
        phase: "completed",
        error_code: "start_failed",
      }),
    });
    const remounted: CoreOperation[] = [];
    await listenForCoreOperation((snapshot) => remounted.push(snapshot));
    handler?.({
      payload: operation({
        operation_id: RETRY_ID,
        sequence: 3,
        phase: "authorizing",
      }),
    });

    expect(delivered.map(({ operationId, sequence }) => [operationId, sequence]))
      .toEqual([[RETRY_ID, 2]]);
    expect(remounted.map(({ operationId, sequence }) => [operationId, sequence]))
      .toEqual([[RETRY_ID, 3]]);
  });
});
