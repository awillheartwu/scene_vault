import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { api, openPathExternal, revealPath, pickFile, pickSavePath, pathJoin, toast } = vi.hoisted(() => ({
  api: {
    getProcessResourceStatus: vi.fn(),
    getStorageResourceStatus: vi.fn(),
    cleanupResource: vi.fn(),
    getLogSettings: vi.fn(),
    updateLogSettings: vi.fn(),
    getDatabaseStartupStatus: vi.fn(),
    preflightDatabase: vi.fn(),
    createDatabaseBackup: vi.fn(),
    stageDatabaseRestore: vi.fn(),
    rebuildDatabaseIndexes: vi.fn(),
    restartAfterDatabaseRestore: vi.fn(),
  },
  openPathExternal: vi.fn(),
  revealPath: vi.fn(),
  pickFile: vi.fn(),
  pickSavePath: vi.fn(),
  pathJoin: (directory: string, name: string) => `${directory}\\${name}`,
  toast: { success: vi.fn(), error: vi.fn(), info: vi.fn() },
}));

vi.mock("@/lib/capture-api", () => ({
  captureApi: api,
  openPathExternal,
  revealPath,
  pickFile,
  pickSavePath,
  pathJoin,
}));

vi.mock("@/lib/toast", () => ({ toast }));

import ResourceStoragePanel from "./ResourceStoragePanel.vue";

const processStatus = {
  capturedAt: "2026-08-11T08:00:00Z",
  approximate: false,
  logicalProcessors: 16,
  groups: [
    {
      role: "rust",
      processCount: 1,
      pids: [10],
      cpuPercent: 1.2,
      workingSetBytes: 100,
      peakWorkingSetBytes: 200,
      privateBytes: 80,
    },
  ],
  totalCpuPercent: 1.2,
  totalWorkingSetBytes: 100,
  totalPrivateBytes: 80,
};

const storageStatus = {
  capturedAt: "2026-08-11T08:00:00Z",
  totalBytes: 100,
  entries: [
    {
      kind: "thumbnail_cache",
      label: "缩略图缓存",
      path: "C:\\cache\\thumbnails",
      totalBytes: 100,
      fileCount: 2,
      cleanupAvailable: true,
      cleanupDescription: "可安全重建",
    },
  ],
};

const startupStatus = {
  mode: "normal" as const,
  databasePath: "C:\\AppData\\SceneVault\\scene-vault.db",
  backupDirectory: "C:\\AppData\\SceneVault\\backups",
  recoveryDirectory: "C:\\AppData\\SceneVault\\recovery",
  errorMessage: null,
  pendingRestore: false,
  restoredOnStartup: false,
};

const preflightReport = {
  ok: true,
  databasePath: startupStatus.databasePath,
  quickCheck: [] as string[],
  foreignKeyIssues: [] as string[],
  migrationIssues: [] as string[],
  schemaVersion: 19,
  sqliteVersion: "3.46.0",
  journalMode: "wal",
  databaseSizeBytes: 1024 * 1024,
  walSizeBytes: 4096,
  checkedAtUtc: "2026-08-11T08:00:00Z",
};

const backupResult = {
  backupPath: "D:\\Backup\\scene-vault-backup-20260811-120000.sqlite",
  manifestPath: "D:\\Backup\\scene-vault-backup-20260811-120000.sqlite.manifest.json",
  manifest: {
    engine: "vacuum-into",
    contentScope: "index-and-metadata-only",
    excludesSourceImages: true,
    backupFile: "scene-vault-backup-20260811-120000.sqlite",
    appVersion: "1.0.0",
    schemaVersion: 19,
    sqliteVersion: "3.46.0",
    createdAtUtc: "2026-08-11T08:00:00Z",
    sha256: "a".repeat(64),
    fileSize: 1024 * 1024,
    tableCounts: { capture_items: 1 },
  },
};

const restoreRequest = {
  engine: "vacuum-into",
  sourceBackupPath: "D:\\Backup\\scene-vault-backup-20260811-120000.sqlite",
  stagedPath: "C:\\AppData\\SceneVault\\recovery\\scene-vault.db.restore-pending",
  manifestPath: "C:\\AppData\\SceneVault\\recovery\\scene-vault.db.restore-pending.manifest.json",
  sha256: "a".repeat(64),
    appVersion: "1.0.0",
  schemaVersion: 19,
  requestedAtUtc: "2026-08-11T08:05:00Z",
};

beforeEach(() => {
  vi.useFakeTimers();
  api.getProcessResourceStatus.mockResolvedValue(processStatus);
  api.getStorageResourceStatus.mockResolvedValue(storageStatus);
  api.cleanupResource.mockResolvedValue({
    kind: "thumbnail_cache",
    removedFiles: 2,
    reclaimedBytes: 100,
    message: "已清理",
  });
  api.getLogSettings.mockResolvedValue({
    retentionDays: 14,
    maxFileSizeMb: 5,
    maxArchivedFiles: 20,
    automaticCleanup: true,
  });
  api.updateLogSettings.mockImplementation(async (settings) => settings);
  api.getDatabaseStartupStatus.mockResolvedValue(startupStatus);
  api.preflightDatabase.mockResolvedValue(preflightReport);
  api.createDatabaseBackup.mockResolvedValue(backupResult);
  api.stageDatabaseRestore.mockResolvedValue(restoreRequest);
  api.rebuildDatabaseIndexes.mockResolvedValue({
    reindexed: true,
    analyzed: true,
    integrityOk: true,
    sqliteVersion: "3.46.0",
    ranAtUtc: "2026-08-11T08:06:00Z",
  });
  api.restartAfterDatabaseRestore.mockResolvedValue(undefined);
  pickSavePath.mockResolvedValue(null);
  pickFile.mockResolvedValue(null);
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("ResourceStoragePanel lifecycle", () => {
  it("uses full-width-safe text actions for refresh controls", async () => {
    const wrapper = mount(ResourceStoragePanel);
    await flushPromises();

    const actions = wrapper.findAll(".section-action");
    expect(actions).toHaveLength(4);
    expect(actions.map((button) => button.text())).toEqual([
      "刷新",
      "数据体检",
      "重新扫描",
      "保存策略",
    ]);
    expect(wrapper.findAll("button.icon-action")).toHaveLength(0);
    expect(wrapper.text()).toContain("Python AI Worker");
    expect(wrapper.text()).toContain("首次 AI 请求时按需启动");
    expect(api.getDatabaseStartupStatus).toHaveBeenCalledTimes(1);
    wrapper.unmount();
  });

  it("samples processes while mounted and stops after unmount", async () => {
    const wrapper = mount(ResourceStoragePanel);
    await flushPromises();
    expect(api.getProcessResourceStatus).toHaveBeenCalledTimes(1);
    expect(api.getStorageResourceStatus).toHaveBeenCalledTimes(1);
    expect(api.getLogSettings).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(2_000);
    await flushPromises();
    expect(api.getProcessResourceStatus).toHaveBeenCalledTimes(2);
    expect(api.getStorageResourceStatus).toHaveBeenCalledTimes(1);

    wrapper.unmount();
    await vi.advanceTimersByTimeAsync(4_000);
    expect(api.getProcessResourceStatus).toHaveBeenCalledTimes(2);
  });

  it("saves the visible log retention and cleanup policy", async () => {
    const wrapper = mount(ResourceStoragePanel);
    await flushPromises();
    const retention = wrapper.get<HTMLInputElement>('.log-policy-fields input[type="number"]');
    await retention.setValue(30);
    await wrapper.get(".log-policy-card").trigger("submit");
    await flushPromises();
    expect(api.updateLogSettings).toHaveBeenCalledWith(expect.objectContaining({
      retentionDays: 30,
      automaticCleanup: true,
    }));
    expect(toast.success).toHaveBeenCalledWith("日志保留与清理策略已保存并立即生效");
  });

  it("confirms cleanup, invokes the generic cleanup interface, and rescans storage", async () => {
    vi.spyOn(window, "confirm").mockReturnValue(true);
    const wrapper = mount(ResourceStoragePanel);
    await flushPromises();

    await wrapper.get(".cleanup-action").trigger("click");
    await flushPromises();

    expect(api.cleanupResource).toHaveBeenCalledWith("thumbnail_cache");
    expect(api.getStorageResourceStatus).toHaveBeenCalledTimes(2);
    expect(toast.success).toHaveBeenCalled();
    wrapper.unmount();
  });
});

describe("ResourceStoragePanel database safety", () => {
  it("runs a preflight and shows the integrity summary", async () => {
    const wrapper = mount(ResourceStoragePanel);
    await flushPromises();

    await wrapper
      .findAll(".section-action")
      .find((button) => button.text().includes("数据体检"))
      ?.trigger("click");
    await flushPromises();

    expect(api.preflightDatabase).toHaveBeenCalledOnce();
    expect(wrapper.text()).toContain("体检通过");
    expect(wrapper.text()).toContain("Schema v19");
    expect(wrapper.text()).toContain("SQLite 3.46.0");
    wrapper.unmount();
  });

  it("creates a backup at the save-dialog destination and shows the result", async () => {
    pickSavePath.mockResolvedValue(backupResult.backupPath);
    const wrapper = mount(ResourceStoragePanel);
    await flushPromises();

    await wrapper
      .findAll(".data-action")
      .find((button) => button.text().includes("立即备份"))
      ?.trigger("click");
    await flushPromises();

    expect(pickSavePath).toHaveBeenCalledWith(
      expect.objectContaining({
        defaultPath: expect.stringMatching(
          /^C:\\AppData\\SceneVault\\backups\\scene-vault-backup-\d{8}-\d{6}\.sqlite$/,
        ),
      }),
    );
    expect(api.createDatabaseBackup).toHaveBeenCalledWith(backupResult.backupPath);
    expect(toast.success).toHaveBeenCalledWith("数据库备份已创建");
    expect(wrapper.text()).toContain("备份已创建");
    wrapper.unmount();
  });

  it("does nothing when the save dialog is cancelled", async () => {
    const wrapper = mount(ResourceStoragePanel);
    await flushPromises();

    await wrapper
      .findAll(".data-action")
      .find((button) => button.text().includes("立即备份"))
      ?.trigger("click");
    await flushPromises();

    expect(api.createDatabaseBackup).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("stages a restore after confirmation and enables restart only then", async () => {
    pickFile.mockResolvedValue(restoreRequest.sourceBackupPath);
    vi.spyOn(window, "confirm").mockReturnValue(true);
    const wrapper = mount(ResourceStoragePanel);
    await flushPromises();

    expect(wrapper.text()).not.toContain("立即重启应用");

    await wrapper
      .findAll(".data-action")
      .find((button) => button.text().includes("从备份恢复"))
      ?.trigger("click");
    await flushPromises();

    expect(window.confirm).toHaveBeenCalledOnce();
    expect(api.stageDatabaseRestore).toHaveBeenCalledWith(restoreRequest.sourceBackupPath);
    expect(toast.success).toHaveBeenCalledWith("备份已校验并暂存，重启后生效");
    expect(wrapper.text()).toContain("恢复已暂存");

    await wrapper
      .findAll(".restart-action")
      .find((button) => button.text().includes("立即重启应用"))
      ?.trigger("click");
    await flushPromises();
    expect(api.restartAfterDatabaseRestore).toHaveBeenCalledOnce();
    wrapper.unmount();
  });

  it("does not stage a restore when the user declines confirmation", async () => {
    pickFile.mockResolvedValue(restoreRequest.sourceBackupPath);
    vi.spyOn(window, "confirm").mockReturnValue(false);
    const wrapper = mount(ResourceStoragePanel);
    await flushPromises();

    await wrapper
      .findAll(".data-action")
      .find((button) => button.text().includes("从备份恢复"))
      ?.trigger("click");
    await flushPromises();

    expect(api.stageDatabaseRestore).not.toHaveBeenCalled();
    expect(wrapper.text()).not.toContain("立即重启应用");
    wrapper.unmount();
  });

  it("rebuilds indexes and reports the maintenance result", async () => {
    const wrapper = mount(ResourceStoragePanel);
    await flushPromises();

    await wrapper
      .findAll(".data-action")
      .find((button) => button.text().includes("重建索引"))
      ?.trigger("click");
    await flushPromises();

    expect(api.rebuildDatabaseIndexes).toHaveBeenCalledOnce();
    expect(toast.success).toHaveBeenCalledWith("索引重建完成，完整性检查通过");
    expect(wrapper.text()).toContain("索引维护完成");
    expect(wrapper.text()).toContain("完整性通过");
    wrapper.unmount();
  });

  it("opens the configured backup directory", async () => {
    const wrapper = mount(ResourceStoragePanel);
    await flushPromises();

    await wrapper
      .findAll(".data-action")
      .find((button) => button.text().includes("打开备份目录"))
      ?.trigger("click");
    await flushPromises();

    expect(openPathExternal).toHaveBeenCalledWith("C:\\AppData\\SceneVault\\backups");
    wrapper.unmount();
  });
});
