import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { CoreOperation } from "../coreOperation";
import { initializeI18n } from "../i18n";
import { CoreOperationPanel } from "./CoreOperationPanel";

const OPERATION_ID = "64c09bda-7afd-4e86-8d61-43bc39a8bc51";

function operation(fields: Partial<CoreOperation> = {}): CoreOperation {
  return {
    schemaVersion: 1,
    operationId: OPERATION_ID,
    sequence: 0,
    operation: "install",
    state: "running",
    phase: "checking_release",
    ...fields,
  };
}

describe("CoreOperationPanel", () => {
  beforeEach(async () => {
    await initializeI18n("en");
  });

  it("translates determinate download progress and technical values", async () => {
    await initializeI18n("zh-CN");

    render(
      <CoreOperationPanel
        operation={operation({
          phase: "downloading",
          currentVersion: "0.1.22",
          targetVersion: "0.1.23",
          bytesCompleted: 512,
          bytesTotal: 1024,
        })}
        onRetry={vi.fn()}
      />,
    );

    expect(
      screen.getByRole("progressbar", { name: "正在下载 WokCore" }),
    ).toHaveAttribute("aria-valuenow", "50");
    expect(
      screen.getByRole("heading", { name: "正在下载 WokCore" }),
    ).toBeInTheDocument();
    expect(screen.getByText(/512 B.*1 KB/)).toBeInTheDocument();
    expect(screen.getByText("当前版本")).toBeInTheDocument();
    expect(screen.getByText("目标版本")).toBeInTheDocument();
    expect(screen.getByText("0.1.22")).toHaveAttribute("dir", "ltr");
    expect(screen.getByText("0.1.23")).toHaveAttribute("dir", "ltr");
  });

  it("translates indeterminate progress and the live phase announcement", async () => {
    await initializeI18n("zh-CN");

    render(
      <CoreOperationPanel
        operation={operation({ phase: "verifying" })}
        onRetry={vi.fn()}
      />,
    );

    expect(
      screen.getByRole("progressbar", { name: "正在验证 WokCore 进度" }),
    ).not.toHaveAttribute("aria-valuenow");
    expect(screen.getByRole("status")).toHaveTextContent("正在验证 WokCore");
  });

  it("translates update rollback and active-request recovery", async () => {
    await initializeI18n("zh-CN");
    const view = render(
      <CoreOperationPanel
        operation={operation({
          operation: "update",
          state: "failed",
          phase: "completed",
          errorCode: "rolled_back",
        })}
        onRetry={vi.fn()}
      />,
    );

    expect(screen.getByText(/已恢复旧版本/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "再次尝试更新" })).toBeEnabled();

    view.rerender(
      <CoreOperationPanel
        operation={operation({
          operation: "update",
          state: "failed",
          phase: "completed",
          activeRequests: 1_000_000,
          errorCode: "active_requests_remain",
        })}
        onRetry={vi.fn()}
      />,
    );
    expect(screen.getByText(/仍有 1,000,000 个活动请求/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "稍后重试更新" })).toBeEnabled();
  });

  it("translates high-priority recovery and diagnostics accessibility", async () => {
    await initializeI18n("zh-CN");

    render(
      <CoreOperationPanel
        operation={operation({
          operation: "update",
          state: "failed",
          phase: "completed",
          errorCode: "recovery_required",
        })}
        onRetry={vi.fn()}
        diagnosticsAvailable
        onOpenDiagnostics={vi.fn()}
      />,
    );

    expect(
      screen.getByRole("heading", { name: "WokCore 需要恢复" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "打开诊断" })).toBeEnabled();
    expect(
      screen.getByText("关闭此窗口不会取消 WokCore 操作。"),
    ).toBeInTheDocument();
  });

  it("keeps unknown backend details private in Simplified Chinese", async () => {
    await initializeI18n("zh-CN");
    const failed = {
      ...operation({ state: "failed", phase: "completed" }),
      errorCode: "C:\\Users\\someone\\token.json",
      rawError: "failed at C:\\private\\wokcore.exe",
    } as CoreOperation;

    render(<CoreOperationPanel operation={failed} onRetry={vi.fn()} />);

    expect(
      screen.getByText("WokRouter 无法安全完成此操作。请检查 WokCore 状态后重试。"),
    ).toBeInTheDocument();
    expect(
      screen.queryByText(/someone|token\.json|private|wokcore\.exe/i),
    ).not.toBeInTheDocument();
  });

  it("updates progress maps when the locale changes without remounting", async () => {
    render(
      <CoreOperationPanel
        operation={operation({ phase: "downloading", bytesCompleted: 1, bytesTotal: 2 })}
        onRetry={vi.fn()}
      />,
    );
    const heading = screen.getByRole("heading", { name: "Downloading WokCore" });
    const progress = screen.getByRole("progressbar", {
      name: "Download WokCore progress",
    });
    const liveRegion = screen.getByRole("status");

    await act(async () => {
      await initializeI18n("zh-CN");
    });

    expect(screen.getByRole("heading", { name: "正在下载 WokCore" })).toBe(heading);
    expect(screen.getByRole("progressbar", { name: "正在下载 WokCore" })).toBe(
      progress,
    );
    expect(screen.getByRole("status")).toBe(liveRegion);
  });

  it("renders real download bytes as a determinate progressbar", () => {
    render(
      <CoreOperationPanel
        operation={operation({
          phase: "downloading",
          targetVersion: "0.1.23",
          bytesCompleted: 512,
          bytesTotal: 1024,
        })}
        onRetry={vi.fn()}
      />,
    );

    const bar = screen.getByRole("progressbar", { name: /download/i });
    expect(bar).toHaveAttribute("aria-valuenow", "50");
    expect(bar).toHaveAttribute("aria-valuemin", "0");
    expect(bar).toHaveAttribute("aria-valuemax", "100");
    expect(screen.getByText(/512 B.*1 KB/i)).toBeInTheDocument();
    expect(screen.getByText("0.1.23")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /cancel/i }),
    ).not.toBeInTheDocument();
  });

  it("uses an indeterminate progressbar outside downloading", () => {
    render(
      <CoreOperationPanel
        operation={operation({
          phase: "verifying",
          currentVersion: "0.1.22",
          targetVersion: "0.1.23",
        })}
        onRetry={vi.fn()}
      />,
    );

    const bar = screen.getByRole("progressbar", { name: /verify/i });
    expect(bar).not.toHaveAttribute("aria-valuenow");
    expect(bar).not.toHaveAttribute("aria-valuemin");
    expect(bar).not.toHaveAttribute("aria-valuemax");
    expect(bar.querySelector(".core-progress__bar--indeterminate")).not.toBeNull();
    expect(screen.getByText("0.1.22")).toBeInTheDocument();
    expect(screen.getByText("0.1.23")).toBeInTheDocument();
  });

  it("keeps one polite live region mounted while announcing phase changes", () => {
    const view = render(
      <CoreOperationPanel
        operation={operation({ phase: "checking_release" })}
        onRetry={vi.fn()}
      />,
    );
    const liveRegion = screen.getByRole("status");
    expect(liveRegion).toHaveAttribute("aria-live", "polite");
    expect(liveRegion).toHaveTextContent("Checking for a WokCore release");

    view.rerender(
      <CoreOperationPanel
        operation={operation({ sequence: 1, phase: "starting" })}
        onRetry={vi.fn()}
      />,
    );

    expect(screen.getByRole("status")).toBe(liveRegion);
    expect(liveRegion).toHaveTextContent("Starting WokCore");
    expect(screen.getAllByRole("status")).toHaveLength(1);
  });

  it("shows safe failure copy and retry without rendering raw bridge text", () => {
    const retry = vi.fn();
    const failed = {
      ...operation({
        state: "failed",
        phase: "completed",
      }),
      errorCode: "C:\\Users\\someone\\token.json",
      rawError: "failed at C:\\private\\wokcore.exe",
    } as CoreOperation;

    render(<CoreOperationPanel operation={failed} onRetry={retry} />);

    expect(
      screen.getByRole("heading", { name: "WokCore setup did not finish" }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/could not complete the operation safely/i),
    ).toBeInTheDocument();
    expect(
      screen.queryByText(/someone|token\.json|private|wokcore\.exe/i),
    ).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(retry).toHaveBeenCalledOnce();
    expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();
  });

  it("maps verified stable failures without exposing implementation details", () => {
    render(
      <CoreOperationPanel
        operation={operation({
          state: "failed",
          phase: "completed",
          errorCode: "invalid_signature",
        })}
        onRetry={vi.fn()}
      />,
    );

    expect(
      screen.getByText(/signature could not be verified/i),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Try again" })).toBeEnabled();
  });

  it("does not offer retry or progress after success", () => {
    render(
      <CoreOperationPanel
        operation={operation({
          state: "succeeded",
          phase: "completed",
          targetVersion: "0.1.23",
        })}
        onRetry={vi.fn()}
      />,
    );

    expect(
      screen.getByRole("heading", { name: "WokCore is ready" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
    expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();
  });

  it("reports a verified update target without using setup success copy", () => {
    render(
      <CoreOperationPanel
        operation={operation({
          operation: "update",
          state: "succeeded",
          phase: "completed",
          currentVersion: "0.1.22",
          targetVersion: "0.1.23",
        })}
        onRetry={vi.fn()}
      />,
    );

    expect(
      screen.getByRole("heading", { name: "WokCore updated" }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/verified WokCore 0\.1\.23/i),
    ).toBeInTheDocument();
  });

  it("reports a stale candidate as current without claiming installation", () => {
    render(
      <CoreOperationPanel
        operation={operation({
          operation: "update",
          state: "succeeded",
          phase: "completed",
          currentVersion: "0.1.22",
        })}
        onRetry={vi.fn()}
      />,
    );

    expect(
      screen.getByRole("heading", {
        name: "WokCore is already current",
      }),
    ).toBeInTheDocument();
    expect(screen.queryByText(/installed successfully/i)).not.toBeInTheDocument();
  });

  it("shows capped active-request context and defers retry", () => {
    render(
      <CoreOperationPanel
        operation={operation({
          operation: "update",
          state: "failed",
          phase: "completed",
          activeRequests: 1_000_000,
          errorCode: "active_requests_remain",
        })}
        onRetry={vi.fn()}
      />,
    );

    expect(
      screen.getByText(
        "1,000,000 active requests remain. WokCore is still serving them; try the update again later.",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Try update later" }),
    ).toBeEnabled();
  });

  it("uses singular English copy for one active request", () => {
    render(
      <CoreOperationPanel
        operation={operation({
          operation: "update",
          state: "failed",
          phase: "completed",
          activeRequests: 1,
          errorCode: "active_requests_remain",
        })}
        onRetry={vi.fn()}
      />,
    );

    expect(
      screen.getByText(
        "1 active request remains. WokCore is still serving it; try the update again later.",
      ),
    ).toBeInTheDocument();
  });

  it("uses natural Simplified Chinese copy for active requests", async () => {
    await initializeI18n("zh-CN");

    render(
      <CoreOperationPanel
        operation={operation({
          operation: "update",
          state: "failed",
          phase: "completed",
          activeRequests: 1,
          errorCode: "active_requests_remain",
        })}
        onRetry={vi.fn()}
      />,
    );

    expect(
      screen.getByText("仍有 1 个活动请求正在处理中。请稍后重试更新。"),
    ).toBeInTheDocument();
  });

  it.each([
    [
      "rolled_back",
      /previous version was restored/i,
    ],
    [
      "update_verification_failed",
      /no untrusted update was installed/i,
    ],
    [
      "update_install_failed",
      /review diagnostics and try again/i,
    ],
  ] as const)("shows safe transactional copy for %s", (errorCode, copy) => {
    render(
      <CoreOperationPanel
        operation={operation({
          operation: "update",
          state: "failed",
          phase: "completed",
          errorCode,
        })}
        onRetry={vi.fn()}
      />,
    );

    expect(screen.getByText(copy)).toBeInTheDocument();
  });

  it("marks recovery_required as high priority and offers diagnostics when available", () => {
    render(
      <CoreOperationPanel
        operation={operation({
          operation: "update",
          state: "failed",
          phase: "completed",
          errorCode: "recovery_required",
        })}
        onRetry={vi.fn()}
        diagnosticsAvailable
        onOpenDiagnostics={vi.fn()}
      />,
    );

    const heading = screen.getByRole("heading", {
      name: "WokCore recovery required",
    });
    expect(heading).toBeInTheDocument();
    expect(heading.closest("section")).toHaveClass(
      "core-operation-panel--urgent",
    );
    expect(
      screen.getByRole("button", { name: "Open diagnostics" }),
    ).toBeEnabled();
  });
});
