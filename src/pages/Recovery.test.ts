import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { api, openPathExternal, pickFile, pathDirectory, revealPath } = vi.hoisted(() => ({
  api: {
    getDatabaseStartupStatus: vi.fn(),
    stageDatabaseRestore: vi.fn(),
    restartAfterDatabaseRestore: vi.fn(),
  },
  openPathExternal: vi.fn(),
  revealPath: vi.fn(),
  pickFile: vi.fn(),
  pathDirectory: (path: string) => path.split(/[\\/]/).slice(0, -1).join("\\") || path,
}));

vi.mock("@/lib/capture-api", () => ({
  captureApi: api,
  openPathExternal,
  revealPath,
  pickFile,
  pathDirectory,
}));

import Recovery from "./Recovery.vue";

const status = {
  mode: "recovery" as const,
  databasePath: "C:\\AppData\\SceneVault\\scene-vault.db",
  backupDirectory: "C:\\AppData\\SceneVault\\backups",
  recoveryDirectory: "C:\\AppData\\SceneVault\\recovery",
  errorMessage: "database disk image is malformed",
  pendingRestore: false,
  restoredOnStartup: false,
};

const restoreRequest = {
  engine: "vacuum-into",
  sourceBackupPath: "D:\\Backup\\scene-vault-backup-20260811-120000.sqlite",
  stagedPath: "C:\\AppData\\SceneVault\\recovery\\scene-vault.db.restore-pending",
  manifestPath: "C:\\AppData\\SceneVault\\recovery\\scene-vault.db.restore-pending.manifest.json",
  sha256: "a".repeat(64),
  appVersion: "0.1.0",
  schemaVersion: 19,
  requestedAtUtc: "2026-08-11T08:05:00Z",
};

beforeEach(() => {
  vi.clearAllMocks();
  api.getDatabaseStartupStatus.mockResolvedValue(status);
  api.stageDatabaseRestore.mockResolvedValue(restoreRequest);
  api.restartAfterDatabaseRestore.mockResolvedValue(undefined);
  pickFile.mockResolvedValue(null);
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("Recovery page", () => {
  it("shows the startup error and data locations in recovery mode", async () => {
    const wrapper = mount(Recovery);
    await flushPromises();

    expect(api.getDatabaseStartupStatus).toHaveBeenCalledOnce();
    expect(wrapper.text()).toContain("数据库恢复");
    expect(wrapper.text()).toContain("database disk image is malformed");
    expect(wrapper.text()).toContain(status.databasePath);
    expect(wrapper.text()).toContain(status.backupDirectory);
    expect(wrapper.text()).toContain(status.recoveryDirectory);
    expect(wrapper.text()).toContain("选择备份文件");
    expect(wrapper.find(".restart-action").exists()).toBe(false);
    wrapper.unmount();
  });

  it("stages a chosen backup after confirmation and only then enables restart", async () => {
    pickFile.mockResolvedValue(restoreRequest.sourceBackupPath);
    vi.spyOn(window, "confirm").mockReturnValue(true);
    const wrapper = mount(Recovery);
    await flushPromises();

    await wrapper
      .findAll(".recovery-action")
      .find((button) => button.text().includes("选择备份文件"))
      ?.trigger("click");
    await flushPromises();

    expect(window.confirm).toHaveBeenCalledOnce();
    expect(api.stageDatabaseRestore).toHaveBeenCalledWith(restoreRequest.sourceBackupPath);
    expect(wrapper.text()).toContain("备份已校验并暂存");
    expect(wrapper.text()).toContain(restoreRequest.sourceBackupPath);

    await wrapper
      .findAll(".restart-action")
      .find((button) => button.text().includes("立即重启应用"))
      ?.trigger("click");
    await flushPromises();
    expect(api.restartAfterDatabaseRestore).toHaveBeenCalledOnce();
    wrapper.unmount();
  });

  it("does not stage when the file picker is cancelled", async () => {
    const wrapper = mount(Recovery);
    await flushPromises();

    await wrapper
      .findAll(".recovery-action")
      .find((button) => button.text().includes("选择备份文件"))
      ?.trigger("click");
    await flushPromises();

    expect(api.stageDatabaseRestore).not.toHaveBeenCalled();
    expect(wrapper.find(".restart-action").exists()).toBe(false);
    wrapper.unmount();
  });

  it("does not stage when the user declines the restore confirmation", async () => {
    pickFile.mockResolvedValue(restoreRequest.sourceBackupPath);
    vi.spyOn(window, "confirm").mockReturnValue(false);
    const wrapper = mount(Recovery);
    await flushPromises();

    await wrapper
      .findAll(".recovery-action")
      .find((button) => button.text().includes("选择备份文件"))
      ?.trigger("click");
    await flushPromises();

    expect(api.stageDatabaseRestore).not.toHaveBeenCalled();
    expect(wrapper.find(".restart-action").exists()).toBe(false);
    wrapper.unmount();
  });

  it("offers restart directly when a restore is already pending", async () => {
    api.getDatabaseStartupStatus.mockResolvedValue({ ...status, pendingRestore: true });
    const wrapper = mount(Recovery);
    await flushPromises();

    expect(wrapper.text()).toContain("检测到已暂存的恢复");
    expect(wrapper.text()).not.toContain("选择备份文件");
    await wrapper
      .findAll(".restart-action")
      .find((button) => button.text().includes("立即重启应用"))
      ?.trigger("click");
    await flushPromises();
    expect(api.restartAfterDatabaseRestore).toHaveBeenCalledOnce();
    wrapper.unmount();
  });

  it("shows the staging error and keeps restart disabled", async () => {
    pickFile.mockResolvedValue(restoreRequest.sourceBackupPath);
    vi.spyOn(window, "confirm").mockReturnValue(true);
    api.stageDatabaseRestore.mockRejectedValue(new Error("checksum mismatch"));
    const wrapper = mount(Recovery);
    await flushPromises();

    await wrapper
      .findAll(".recovery-action")
      .find((button) => button.text().includes("选择备份文件"))
      ?.trigger("click");
    await flushPromises();

    expect(wrapper.text()).toContain("checksum mismatch");
    expect(wrapper.find(".restart-action").exists()).toBe(false);
    wrapper.unmount();
  });

  it("opens the data directory containing the database", async () => {
    const wrapper = mount(Recovery);
    await flushPromises();

    await wrapper
      .findAll(".recovery-action")
      .find((button) => button.text().includes("打开数据目录"))
      ?.trigger("click");
    await flushPromises();

    expect(openPathExternal).toHaveBeenCalledWith("C:\\AppData\\SceneVault");
    wrapper.unmount();
  });

  it("surfaces a failure to read the startup status", async () => {
    api.getDatabaseStartupStatus.mockRejectedValue(new Error("ipc unavailable"));
    const wrapper = mount(Recovery);
    await flushPromises();

    expect(wrapper.text()).toContain("ipc unavailable");
    expect(wrapper.find(".restart-action").exists()).toBe(false);
    wrapper.unmount();
  });
});
