import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  modelPull: vi.fn(),
  modelPullCancel: vi.fn().mockResolvedValue(undefined),
  modelSetDefault: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("@tauri-apps/api/core", () => ({
  Channel: class { onmessage?: unknown },
  invoke: vi.fn(),
}));

vi.mock("../tauri", () => ({
  characterList: vi.fn(),
  modelPull: mocks.modelPull,
  modelPullCancel: mocks.modelPullCancel,
  modelSetDefault: mocks.modelSetDefault,
  settingSet: vi.fn(),
}));

import { useApp } from "../store";

describe("model download cancellation", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.modelPullCancel.mockResolvedValue(undefined);
    mocks.modelSetDefault.mockResolvedValue(undefined);
    useApp.setState({ model_download: null });
  });

  it("does not select or complete a pull cancelled while its IPC call is pending", async () => {
    let finishPull!: () => void;
    mocks.modelPull.mockImplementationOnce(() => new Promise<void>((resolve) => { finishPull = resolve; }));

    const download = useApp.getState().startModelDownload("qwen3:8b", "chat");
    await Promise.resolve();
    await useApp.getState().cancelModelDownload();
    finishPull();
    await download;

    expect(mocks.modelPullCancel).toHaveBeenCalledWith("qwen3:8b");
    expect(mocks.modelSetDefault).not.toHaveBeenCalled();
    expect(useApp.getState().model_download).toMatchObject({ phase: "cancelled" });
  });
});
