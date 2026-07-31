import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { CoreOperation } from "../coreOperation";
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
});
