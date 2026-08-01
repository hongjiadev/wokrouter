import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { z } from "zod";

const operationSchemaVersion = 1;
const maxActiveRequests = 1_000_000;
const maxRetiredOperationIds = 64;
const safeUnsignedInteger = z
  .number()
  .int()
  .min(0)
  .max(Number.MAX_SAFE_INTEGER);
const semverSchema = z
  .string()
  .regex(
    /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/,
  );
const operationKindSchema = z.enum(["install", "update"]);
const operationStateSchema = z.enum(["running", "succeeded", "failed"]);
const operationPhaseSchema = z.enum([
  "checking_release",
  "downloading",
  "verifying",
  "installing",
  "preparing_service",
  "draining",
  "stopping",
  "starting",
  "authorizing",
  "verifying_runtime",
  "rolling_back",
  "completed",
]);
const installErrorSchema = z.enum([
  "download_failed",
  "invalid_install_state",
  "invalid_manifest",
  "invalid_signature",
  "incompatible_manifest",
  "artifact_size_mismatch",
  "artifact_hash_mismatch",
  "invalid_archive",
  "unsafe_install_location",
  "install_in_progress",
  "install_failed",
  "install_record_failed",
  "start_failed",
  "authorization_failed",
  "invalid_progress",
]);
const updateErrorSchema = z.enum([
  "update_unavailable",
  "incompatible_manifest",
  "update_verification_failed",
  "update_install_failed",
  "active_requests_remain",
  "rolled_back",
  "recovery_required",
  "operation_in_progress",
  "invalid_progress",
]);

const installPhases = new Set<z.infer<typeof operationPhaseSchema>>([
  "checking_release",
  "downloading",
  "verifying",
  "installing",
  "starting",
  "authorizing",
  "verifying_runtime",
  "completed",
]);
const updatePhases = new Set<z.infer<typeof operationPhaseSchema>>([
  "checking_release",
  "downloading",
  "verifying",
  "installing",
  "preparing_service",
  "draining",
  "stopping",
  "starting",
  "verifying_runtime",
  "rolling_back",
  "completed",
]);

const rawCoreOperationSchema = z
  .object({
    schema_version: z.literal(operationSchemaVersion),
    operation_id: z.uuid(),
    sequence: safeUnsignedInteger,
    operation: operationKindSchema,
    state: operationStateSchema,
    phase: operationPhaseSchema,
    current_version: semverSchema.optional(),
    target_version: semverSchema.optional(),
    bytes_completed: safeUnsignedInteger.optional(),
    bytes_total: safeUnsignedInteger.positive().optional(),
    active_requests: safeUnsignedInteger.max(maxActiveRequests).optional(),
    error_code: z.string().optional(),
  })
  .passthrough()
  .superRefine((value, context) => {
    const phaseAllowed =
      value.operation === "install"
        ? installPhases.has(value.phase)
        : updatePhases.has(value.phase);
    if (!phaseAllowed) {
      context.addIssue({
        code: "custom",
        message: "phase is invalid for operation",
        path: ["phase"],
      });
    }

    const hasCompletedBytes = value.bytes_completed !== undefined;
    const hasTotalBytes = value.bytes_total !== undefined;
    const validBytes =
      value.phase === "downloading"
        ? hasCompletedBytes &&
          hasTotalBytes &&
          value.bytes_completed! <= value.bytes_total!
        : !hasCompletedBytes && !hasTotalBytes;
    if (!validBytes) {
      context.addIssue({
        code: "custom",
        message: "byte progress is invalid for phase",
        path: ["bytes_completed"],
      });
    }

    if (
      value.active_requests !== undefined &&
      value.operation !== "update"
    ) {
      context.addIssue({
        code: "custom",
        message: "active requests are invalid for operation",
        path: ["active_requests"],
      });
    }

    const errorSchema =
      value.operation === "install" ? installErrorSchema : updateErrorSchema;
    const errorIsValid =
      value.error_code === undefined ||
      errorSchema.safeParse(value.error_code).success;
    const validState =
      (value.state === "running" &&
        value.phase !== "completed" &&
        value.error_code === undefined) ||
      (value.state === "succeeded" &&
        value.phase === "completed" &&
        value.error_code === undefined) ||
      (value.state === "failed" &&
        value.error_code !== undefined &&
        errorIsValid);
    if (!validState) {
      context.addIssue({
        code: "custom",
        message: "terminal state is invalid",
        path: ["state"],
      });
    }
  })
  .transform((value) => ({
    schemaVersion: value.schema_version,
    operationId: value.operation_id,
    sequence: value.sequence,
    operation: value.operation,
    state: value.state,
    phase: value.phase,
    ...(value.current_version === undefined
      ? {}
      : { currentVersion: value.current_version }),
    ...(value.target_version === undefined
      ? {}
      : { targetVersion: value.target_version }),
    ...(value.bytes_completed === undefined
      ? {}
      : { bytesCompleted: value.bytes_completed }),
    ...(value.bytes_total === undefined
      ? {}
      : { bytesTotal: value.bytes_total }),
    ...(value.active_requests === undefined
      ? {}
      : { activeRequests: value.active_requests }),
    ...(value.error_code === undefined
      ? {}
      : { errorCode: value.error_code }),
  }));

const rawCoreUpdateCheckSchema = z
  .object({
    code: z.enum(["current", "update_available"]),
    current_version: semverSchema,
    target_version: semverSchema.optional(),
  })
  .passthrough()
  .superRefine((value, context) => {
    const targetIsValid =
      (value.code === "current" && value.target_version === undefined) ||
      (value.code === "update_available" &&
        value.target_version !== undefined);
    if (!targetIsValid) {
      context.addIssue({
        code: "custom",
        message: "target version does not match update result",
        path: ["target_version"],
      });
    }
  })
  .transform((value) => ({
    code: value.code,
    currentVersion: value.current_version,
    ...(value.target_version === undefined
      ? {}
      : { targetVersion: value.target_version }),
  }));

export type CoreOperation = z.infer<typeof rawCoreOperationSchema>;
export type CoreOperationSnapshot = CoreOperation;
export type CoreUpdateCheck = z.infer<typeof rawCoreUpdateCheckSchema>;

export function parseCoreOperation(value: unknown): CoreOperation {
  const parsed = rawCoreOperationSchema.safeParse(value);
  if (!parsed.success) {
    throw new Error("Invalid WokCore operation returned by desktop bridge.", {
      cause: parsed.error,
    });
  }
  return parsed.data;
}

export function parseCoreUpdateCheck(value: unknown): CoreUpdateCheck {
  const parsed = rawCoreUpdateCheckSchema.safeParse(value);
  if (!parsed.success) {
    throw new Error("Invalid WokCore update check returned by desktop bridge.", {
      cause: parsed.error,
    });
  }
  return parsed.data;
}

type DownloadProgress = {
  completed: number;
  total: number;
};

let bridgeRevision = 0;
let lastAuthority:
  | { snapshot: CoreOperation | null; revision: number }
  | undefined;
const operationTrackers = new Set<OperationTracker>();

function nextRevision(): number {
  bridgeRevision += 1;
  return bridgeRevision;
}

function isTerminal(operation: CoreOperation): boolean {
  return operation.state !== "running";
}

function downloadProgress(
  operation: CoreOperation,
): DownloadProgress | undefined {
  return operation.phase === "downloading"
    ? {
        completed: operation.bytesCompleted!,
        total: operation.bytesTotal!,
      }
    : undefined;
}

function downloadFollows(
  previous: DownloadProgress | undefined,
  operation: CoreOperation,
): boolean {
  if (operation.phase !== "downloading" || previous === undefined) {
    return true;
  }
  return (
    operation.bytesTotal === previous.total &&
    operation.bytesCompleted! >= previous.completed
  );
}

class OperationTracker {
  private current: CoreOperation | undefined;
  private download: DownloadProgress | undefined;
  private readonly retiredOperationIds = new Set<string>();
  private readonly retiredOperationOrder: string[] = [];
  private revision = 0;

  constructor(
    private readonly listener: (operation: CoreOperation) => void,
    authority = lastAuthority,
  ) {
    if (authority?.snapshot) {
      this.replace(authority.snapshot, authority.revision);
    } else if (authority) {
      this.revision = authority.revision;
    }
  }

  acceptEvent(operation: CoreOperation): number | undefined {
    if (this.retiredOperationIds.has(operation.operationId)) {
      return undefined;
    }
    if (!this.current) {
      const revision = nextRevision();
      this.replace(operation, revision);
      return revision;
    }
    if (operation.operationId === this.current.operationId) {
      if (
        isTerminal(this.current) ||
        operation.operation !== this.current.operation ||
        operation.sequence <= this.current.sequence ||
        !downloadFollows(this.download, operation)
      ) {
        return undefined;
      }
      const revision = nextRevision();
      this.replace(operation, revision, false);
      return revision;
    }
    if (isTerminal(this.current) && operation.state === "running") {
      const revision = nextRevision();
      this.replace(operation, revision);
      return revision;
    }
    return undefined;
  }

  acceptAuthority(
    operation: CoreOperation | null,
    revision: number,
  ): void {
    if (operation === null) {
      if (revision >= this.revision) {
        if (this.current) {
          this.retire(this.current.operationId);
        }
        this.current = undefined;
        this.download = undefined;
        this.revision = revision;
      }
      return;
    }
    if (operation.operationId === this.current?.operationId) {
      this.revision = Math.max(this.revision, revision);
      if (
        operation.sequence > this.current.sequence &&
        !isTerminal(this.current) &&
        operation.operation === this.current.operation &&
        downloadFollows(this.download, operation)
      ) {
        this.replace(operation, this.revision, false);
        this.listener(operation);
      }
      return;
    }
    if (revision >= this.revision) {
      this.replace(operation, revision);
      this.listener(operation);
    }
  }

  deliver(operation: CoreOperation): void {
    this.listener(operation);
  }

  private replace(
    operation: CoreOperation,
    revision: number,
    resetDownload = true,
  ): void {
    if (this.current && this.current.operationId !== operation.operationId) {
      this.retire(this.current.operationId);
    }
    this.current = operation;
    this.revision = revision;
    if (resetDownload) {
      this.download = undefined;
    }
    this.download = downloadProgress(operation) ?? this.download;
  }

  private retire(operationId: string): void {
    if (this.retiredOperationIds.has(operationId)) {
      return;
    }
    this.retiredOperationIds.add(operationId);
    this.retiredOperationOrder.push(operationId);
    if (this.retiredOperationOrder.length > maxRetiredOperationIds) {
      const oldest = this.retiredOperationOrder.shift();
      if (oldest !== undefined) {
        this.retiredOperationIds.delete(oldest);
      }
    }
  }
}

function applyAuthority(
  snapshot: CoreOperation | null,
  revision: number,
): { snapshot: CoreOperation | null; revision: number } {
  const authority = recordKnownSnapshot(snapshot, revision);
  for (const tracker of operationTrackers) {
    tracker.acceptAuthority(authority.snapshot, authority.revision);
  }
  return authority;
}

function recordKnownSnapshot(
  snapshot: CoreOperation | null,
  revision: number,
): { snapshot: CoreOperation | null; revision: number } {
  const known = lastAuthority;
  if (
    snapshot !== null &&
    known?.snapshot !== null &&
    known?.snapshot !== undefined &&
    snapshot.operationId === known.snapshot.operationId
  ) {
    const next = {
      snapshot:
        snapshot.sequence > known.snapshot.sequence
          ? snapshot
          : known.snapshot,
      revision: Math.max(known.revision, revision),
    };
    lastAuthority = next;
    return next;
  }
  if (known === undefined || revision >= known.revision) {
    const next = { snapshot, revision };
    lastAuthority = next;
    return next;
  }
  return known;
}

async function invokeOperation(
  command: "install_and_start_core" | "install_core_update",
  args?: Record<string, unknown>,
): Promise<CoreOperation> {
  const revision = nextRevision();
  const value =
    args === undefined
      ? await invoke<unknown>(command)
      : await invoke<unknown>(command, args);
  const operation = parseCoreOperation(value);
  const authority = applyAuthority(operation, revision);
  if (authority.snapshot === null) {
    throw new Error("WokCore operation is no longer current.");
  }
  return authority.snapshot;
}

export async function getCoreOperation(): Promise<CoreOperation | null> {
  const revision = nextRevision();
  const value = await invoke<unknown>("core_operation_status");
  if (value === null) {
    return applyAuthority(null, revision).snapshot;
  }
  const operation = parseCoreOperation(value);
  return applyAuthority(operation, revision).snapshot;
}

export function installAndStartCore(): Promise<CoreOperation> {
  return invokeOperation("install_and_start_core");
}

export async function checkCoreUpdate(): Promise<CoreUpdateCheck> {
  return parseCoreUpdateCheck(await invoke<unknown>("check_core_update"));
}

let startupUpdateCheck: Promise<CoreUpdateCheck> | undefined;

export function checkCoreUpdateOnce(): Promise<CoreUpdateCheck> {
  startupUpdateCheck ??= checkCoreUpdate();
  return startupUpdateCheck;
}

export function retryCoreUpdateCheck(): Promise<CoreUpdateCheck> {
  startupUpdateCheck = checkCoreUpdate();
  return startupUpdateCheck;
}

export function rememberCoreUpdateCompletion(
  operation: CoreOperation,
): void {
  if (
    operation.operation !== "update" ||
    operation.state !== "succeeded" ||
    operation.phase !== "completed"
  ) {
    return;
  }
  const currentVersion =
    operation.targetVersion ?? operation.currentVersion;
  if (currentVersion !== undefined) {
    startupUpdateCheck = Promise.resolve({
      code: "current",
      currentVersion,
    });
  }
}

export function installCoreUpdate(
  expectedVersion: string,
): Promise<CoreOperation> {
  const parsedVersion = semverSchema.safeParse(expectedVersion);
  if (!parsedVersion.success) {
    return Promise.reject(new Error("Invalid expected WokCore version."));
  }
  return invokeOperation("install_core_update", {
    expectedVersion: parsedVersion.data,
  });
}

export async function listenForCoreOperation(
  listener: (operation: CoreOperation) => void,
): Promise<UnlistenFn> {
  const tracker = new OperationTracker(listener);
  operationTrackers.add(tracker);
  let unlisten: UnlistenFn;
  try {
    unlisten = await listen<unknown>("core-operation-progress", (event) => {
      let operation: CoreOperation;
      try {
        operation = parseCoreOperation(event.payload);
      } catch {
        return;
      }
      const revision = tracker.acceptEvent(operation);
      if (revision !== undefined) {
        recordKnownSnapshot(operation, revision);
        tracker.deliver(operation);
      }
    });
  } catch (error) {
    operationTrackers.delete(tracker);
    throw error;
  }
  let listening = true;
  return () => {
    if (!listening) {
      return;
    }
    listening = false;
    operationTrackers.delete(tracker);
    unlisten();
  };
}
