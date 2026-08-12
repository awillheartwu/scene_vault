import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await tauriInvoke<T>(command, args);
  } catch (error) {
    if (command !== "record_client_event") {
      void tauriInvoke("record_client_event", {
        input: {
          level: "error",
          module: "ipc",
          event: "command_failed",
          message: `${command} failed`,
          operationId: command,
          outcome: "failed",
          errorCode: "tauri_invoke_error",
        },
      }).catch(() => undefined);
    }
    throw error;
  }
}

export interface Project {
  id: string;
  name: string;
  description: string | null;
  coverAssetId: string | null;
  coverCaptureItemId: string | null;
  lastSourceDirectory: string | null;
  lastDestinationDirectory: string | null;
  destinationDirectory: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface ProjectOverviewSummary {
  projectId: string;
  name: string;
  description: string | null;
  createdAt: string;
  sourceCount: number;
  destinationConfigured: boolean;
  sessionCount: number;
  activeSessionCount: number;
  captureCount: number;
  awaitingCount: number;
  processingCount: number;
  completedCount: number;
  failedCount: number;
  lastActivityAt: string;
  latestCaptureItemId: string | null;
  coverCaptureItemId: string | null;
}

export interface ProjectDeletionPreview {
  captureCount: number;
  characterCount: number;
  sessionCount: number;
  hasActiveSession: boolean;
  noteCount: number;
  collectionCount: number;
  orphanAssetCount: number;
}

export interface ProjectSourceDirectory {
  id: string;
  projectId: string;
  directory: string;
  enabled: boolean;
  createdAt: string;
}

export interface Character {
  id: string;
  projectId: string;
  name: string;
  aliasesJson: string;
  avatarAssetId: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface CharacterSummary {
  id: string;
  name: string;
  aliasesJson: string;
  avatarAssetId: string | null;
  avatarCaptureItemId: string | null;
  captureCount: number;
  pendingReviewCount: number;
  sampleCount: number;
  degradedCount: number;
  lastCapturedAt: string | null;
  latestCaptureItemId: string | null;
  latestAvatarPath: string | null;
}

export interface FaceSample {
  id: string;
  characterId: string;
  captureItemId: string;
  faceBoxJson: string | null;
  confidence: number | null;
  status: "active" | "revoked";
  flagged: number;
  createdAt: string;
  updatedAt: string;
}

export interface FaceBankRebuildSummary {
  total: number;
  rebuilt: number;
  noFace: number;
  notEnrolled: number;
  skippedMissingSource: number;
  failed: number;
  stalePreserved: number;
  suggestionsRefreshed: number;
}

export interface VerificationResult {
  score: number | null;
  bestOtherScore: number | null;
  bestOtherCharacterId: string | null;
  hasSamples: boolean;
  hasFeature: boolean;
  level: "ok" | "low" | "strong" | "unverified";
}

export interface DetectionSettings {
  scoreThreshold: number | null;
  nmsThreshold: number | null;
  topK: number | null;
  areaWeight: number | null;
  confidenceWeight: number | null;
  centerWeight: number | null;
  sharpnessWeight: number | null;
  edgePenaltyWeight: number | null;
  edgeMarginRatio: number | null;
  minSharpness: number | null;
  blurPenaltyWeight: number | null;
}

export interface AnnotationSettings {
  textColor: [number, number, number] | null;
  strokeColor: [number, number, number] | null;
  strokeWidth: number | null;
  padding: number | null;
  faceBoxExpansion: number | null;
  faceTextPosition: string | null;
  fallbackPosition: string | null;
  textOffsetX: number | null;
  textOffsetY: number | null;
  fontSize: number | null;
}

export interface CropSettings {
  aspectRatio: string | null;
  scaleX: number | null;
  scaleTop: number | null;
  scaleBottom: number | null;
  minSize: number | null;
}

export interface ProcessingSettings {
  detection: DetectionSettings | null;
  annotation: AnnotationSettings | null;
  crop: CropSettings | null;
}

export interface CaptureSession {
  id: string;
  projectId: string;
  status: "active" | "completed" | "cancelled";
  startedAt: string;
  endedAt: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface StartCaptureSessionResult {
  session: CaptureSession;
  stoppedProjects: string[];
}

export interface UnimportedCapture {
  path: string;
  fileSize: number;
  modifiedAtMs: number | null;
}

export type CaptureStatus =
  | "awaiting_label"
  | "queued"
  | "processing"
  | "archive_pending"
  | "completed"
  | "failed";

export type CaptureClassification = "unclassified" | "person" | "scene" | "private";

export interface CaptureItem {
  id: string;
  projectId: string;
  sessionId: string;
  assetId: string | null;
  characterId: string | null;
  classification: CaptureClassification;
  sourcePath: string;
  fileSize: number | null;
  modifiedAtMs: number | null;
  contentHash: string | null;
  annotatedPath: string | null;
  avatarPath: string | null;
  destinationPath: string | null;
  destinationAvatarPath: string | null;
  status: CaptureStatus;
  faceBoxJson: string | null;
  faceCount: number | null;
  suggestedCharacterId: string | null;
  recognitionConfidence: number | null;
  recognitionSource: "face_bank" | "vision" | "manual" | null;
  reviewStatus: "none" | "pending" | "accepted" | "rejected";
  errorMessage: string | null;
  failureStage: "processing" | "archive" | null;
  attemptCount: number;
  nextRetryAt: string | null;
  processingWarningsJson: string;
  capturedAt: string;
  processedAt: string | null;
  archivedAt: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface CaptureHistoryEntry extends CaptureItem {
  projectId: string;
  projectName: string;
  sessionStatus: string;
  characterName: string | null;
}

export interface CaptureHistoryPage {
  entries: CaptureHistoryEntry[];
  total: number;
}

export interface CaptureItemPage {
  items: CaptureItem[];
  total: number;
  page: number;
  pageSize: number;
}

export type LogLevel = "debug" | "info" | "warn" | "error";

export interface LogRecord {
  timestamp: string;
  level: LogLevel;
  module: string;
  message: string;
  event?: string | null;
  operationId?: string | null;
  requestId?: string | null;
  projectId?: string | null;
  sessionId?: string | null;
  captureItemId?: string | null;
  noteId?: string | null;
  attempt?: number | null;
  durationMs?: number | null;
  outcome?: string | null;
  workerMode?: string | null;
  errorCode?: string | null;
}

export interface LogQueryResult {
  records: LogRecord[];
  matchedCount: number;
  truncated: boolean;
  offset: number;
  nextOffset: number | null;
  hasMore: boolean;
}

export interface LogPolicySettings {
  retentionDays: number;
  maxFileSizeMb: number;
  maxArchivedFiles: number;
  automaticCleanup: boolean;
}

export interface LogStatus {
  directory: string;
  fileCount: number;
  totalBytes: number;
  retentionDays: number;
  maxFileBytes: number;
  maxArchivedFiles: number;
  droppedRecords: number;
  automaticCleanup: boolean;
}

export interface LogCleanupResult {
  removedFiles: number;
  status: LogStatus;
}

export interface ProcessResourceGroup {
  role: "rust" | "webview" | "python" | "helper";
  processCount: number;
  pids: number[];
  cpuPercent: number | null;
  workingSetBytes: number;
  peakWorkingSetBytes: number;
  privateBytes: number | null;
}

export interface ProcessResourceStatus {
  capturedAt: string;
  approximate: boolean;
  logicalProcessors: number;
  groups: ProcessResourceGroup[];
  totalCpuPercent: number | null;
  totalWorkingSetBytes: number;
  totalPrivateBytes: number | null;
}

export interface StorageResourceEntry {
  kind: string;
  label: string;
  path: string | null;
  totalBytes: number;
  fileCount: number;
  cleanupAvailable: boolean;
  cleanupDescription: string | null;
}

export interface StorageResourceStatus {
  capturedAt: string;
  entries: StorageResourceEntry[];
  totalBytes: number;
}

export interface CleanupResourceResult {
  kind: string;
  removedFiles: number;
  reclaimedBytes: number;
  message: string;
}

export interface DatabaseStartupStatus {
  mode: "normal" | "recovery";
  databasePath: string;
  backupDirectory: string;
  recoveryDirectory: string;
  errorMessage: string | null;
  pendingRestore: boolean;
  restoredOnStartup: boolean;
}

export interface PreflightReport {
  ok: boolean;
  databasePath: string;
  quickCheck: string[];
  foreignKeyIssues: string[];
  migrationIssues: string[];
  schemaVersion: number | null;
  sqliteVersion: string;
  journalMode: string;
  databaseSizeBytes: number;
  walSizeBytes: number;
  checkedAtUtc: string;
}

export interface BackupManifest {
  engine: string;
  contentScope: string;
  excludesSourceImages: boolean;
  backupFile: string;
  appVersion: string;
  schemaVersion: number | null;
  sqliteVersion: string;
  createdAtUtc: string;
  sha256: string;
  fileSize: number;
  tableCounts: Record<string, number>;
}

export interface DatabaseBackupResult {
  backupPath: string;
  manifestPath: string;
  manifest: BackupManifest;
}

export interface RestoreRequest {
  engine: string;
  sourceBackupPath: string;
  stagedPath: string;
  manifestPath: string;
  sha256: string;
  appVersion: string;
  schemaVersion: number | null;
  requestedAtUtc: string;
}

export interface MaintenanceReport {
  reindexed: boolean;
  analyzed: boolean;
  integrityOk: boolean;
  sqliteVersion: string;
  ranAtUtc: string;
}

export interface VisionSettings {
  pythonExecutablePath: string | null;
  pythonModuleRoot: string | null;
  yunetModelPath: string | null;
  sfaceModelPath: string | null;
  recognizer: "sface" | "arcface" | null;
  arcfaceModelPath: string | null;
  fontPath: string | null;
}

export interface BundledFont {
  name: string;
  path: string;
}

export interface RecognitionSettings {
  extractAtRegistration: boolean;
  verificationEnabled: boolean;
  profiles: Record<string, ModelRecognitionProfile>;
  /** Faces sharper than this are enrolled into the Face Bank; null = no
   * gate. Default 3.0 keeps printed/photo faces out. */
  minSampleSharpness: number | null;
  /** Minimum share of the image the primary face must occupy (0..1);
   * null = no gate. Default 0.005. */
  minFaceAreaRatio: number | null;
  /** Reserved (not enforced yet): minimum detection confidence. */
  minFaceConfidence: number | null;
}

export interface ModelRecognitionProfile {
  confidenceThreshold: number | null;
  margin: number | null;
  verificationHighThreshold: number | null;
  verificationLowThreshold: number | null;
  crossCheckDelta: number | null;
}

export interface FaceBankModelStatus {
  bankModelId: string | null;
  bankModelVersion: string | null;
  activeModelId: string | null;
  activeModelVersion: string | null;
  sampleCount: number;
  incompatibleSampleCount: number;
  compatible: boolean;
  modelCounts: Array<{
    modelId: string;
    modelVersion: string;
    sampleCount: number;
    compatible: boolean;
  }>;
}

export interface ResolvedRecognitionProfile {
  confidenceThreshold: number;
  margin: number;
  verificationHighThreshold: number;
  verificationLowThreshold: number;
  crossCheckDelta: number;
}

export interface VisionHealth {
  status: string;
  engineVersion: string | null;
  pythonVersion: string | null;
  processScreenshotAvailable: boolean;
  errorMessage: string | null;
}

export interface CaptureRuntimeStatus {
  engineStatus: string;
  workerStatus: string;
  activeCaptureItemId: string | null;
  activeCaptureSourcePath: string | null;
  queuedCount: number;
  archivePendingCount: number;
  prelabelPendingCount: number;
  lastError: string | null;
}

export interface AppSettings {
  classifyShortcut: string;
  noteShortcut: string;
  showPrivateByDefault: boolean;
  autoSaveNotes: boolean;
  splitPopupWindows: boolean;
  autoCloseEmptyPopup: boolean;
  thumbnailCacheSizeMb: number;
  thumbnailGenerationConcurrency: number;
}

export interface ThumbnailCacheStatus {
  dir: string;
  totalBytes: number;
  limitBytes: number;
}

export interface ArchiveNamingSettings {
  template: string;
  separator: string;
}

export interface ClassifyPopupContext {
  items: CaptureItem[];
  projectId: string | null;
  projectName: string | null;
  characters: Character[];
}

export interface ProjectNote {
  id: string;
  projectId: string;
  destinationDirectory: string;
  remotePath: string;
  content: string;
  status: "synced" | "pending" | "failed";
  attemptCount: number;
  nextRetryAt: string | null;
  errorMessage: string | null;
  syncedAt: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface OpenProjectNoteResult {
  opened: boolean;
  path: string;
}

export const captureApi = {
  listProjects: () => invoke<Project[]>("list_projects"),
  listProjectOverviews: () =>
    invoke<ProjectOverviewSummary[]>("list_project_overviews"),
  renameProject: (projectId: string, name: string) =>
    invoke<Project>("rename_project", { input: { projectId, name } }),
  previewProjectDelete: (projectId: string) =>
    invoke<ProjectDeletionPreview>("preview_project_deletion", { projectId }),
  deleteProject: (projectId: string) =>
    invoke<ProjectDeletionPreview>("delete_project", { projectId }),
  createProject: (name: string) =>
    invoke<Project>("create_project", {
      input: { name, description: null, coverAssetId: null },
    }),
  listCharacters: (projectId: string) =>
    invoke<Character[]>("list_characters", { projectId }),
  createCharacter: (projectId: string, name: string) =>
    invoke<Character>("create_character", {
      input: { projectId, name, aliasesJson: null },
    }),
  renameCharacter: (input: { characterId: string; name: string }) =>
    invoke<Character>("rename_character", { input }),
  mergeCharacters: (input: {
    sourceCharacterId: string;
    targetCharacterId: string;
  }) => invoke<Character>("merge_characters", { input }),
  setCharacterAvatar: (input: {
    characterId: string;
    avatarAssetId: string | null;
  }) => invoke<Character>("set_character_avatar", { input }),
  listProjectCharacterSummaries: (projectId: string) =>
    invoke<CharacterSummary[]>("list_project_character_summaries", {
      projectId,
    }),
  listSessions: (projectId: string) =>
    invoke<CaptureSession[]>("list_capture_sessions", { projectId }),
  startSession: (projectId: string) =>
    invoke<StartCaptureSessionResult>("start_capture_session", {
      input: { projectId },
    }),
  listProjectRecentCaptures: (projectId: string, limit?: number) =>
    invoke<CaptureItem[]>("list_project_recent_captures", {
      projectId,
      limit,
    }),
  listSourceDirectories: (projectId: string) =>
    invoke<ProjectSourceDirectory[]>("list_project_source_directories", {
      projectId,
    }),
  addSourceDirectory: (projectId: string, directory: string) =>
    invoke<ProjectSourceDirectory>("add_project_source_directory", {
      input: { projectId, directory },
    }),
  removeSourceDirectory: (id: string) =>
    invoke<void>("remove_project_source_directory", { input: { id } }),
  setSourceDirectoryEnabled: (id: string, enabled: boolean) =>
    invoke<ProjectSourceDirectory>("set_project_source_directory_enabled", {
      input: { id, enabled },
    }),
  setProjectDestination: (projectId: string, directory: string) =>
    invoke<Project>("set_project_destination_directory", {
      input: { projectId, directory },
    }),
  setProjectCover: (projectId: string, captureItemId: string | null) =>
    invoke<Project>("set_project_cover", {
      input: { projectId, captureItemId },
    }),
  endSession: (sessionId: string) =>
    invoke<CaptureSession>("end_capture_session", {
      input: { sessionId, status: "completed" },
    }),
  listItems: (sessionId: string) =>
    invoke<CaptureItem[]>("list_capture_items", { sessionId }),
  listUnimportedCaptures: (sessionId: string) =>
    invoke<UnimportedCapture[]>("list_unimported_captures", { sessionId }),
  importDirectoryCaptures: (sessionId: string, paths: string[]) =>
    invoke<number>("import_directory_captures", {
      input: { sessionId, paths },
    }),
  deferredImportRecognitionCount: (sessionId: string) =>
    invoke<number>("deferred_import_recognition_count", {
      input: { sessionId },
    }),
  startImportedRecognition: (sessionId: string) =>
    invoke<number>("start_imported_recognition", {
      input: { sessionId },
    }),
  label: (
    captureItemId: string,
    characterId: string | null,
    classification: CaptureClassification = "person",
  ) =>
    invoke<CaptureItem>("label_capture", {
      input: { captureItemId, characterId, classification },
    }),
  relabel: (
    captureItemId: string,
    characterId: string | null,
    classification: CaptureClassification = "person",
  ) =>
    invoke<CaptureItem>("relabel_capture_item", {
      input: { captureItemId, characterId, classification },
    }),
  retry: (captureItemId: string) =>
    invoke<CaptureItem>("retry_capture", { input: { captureItemId } }),
  refreshCaptureFaceFeature: (captureItemId: string) =>
    invoke<CaptureItem>("refresh_capture_face_feature", {
      input: { captureItemId },
    }),
  retryDegradedCaptures: (
    projectId: string,
    characterId: string | null,
  ) => invoke<number>("retry_degraded_captures", {
    input: { projectId, characterId },
  }),
  listCategoryItems: (input: {
    projectId: string;
    category: "unclassified" | "scene" | "private";
  }) => invoke<CaptureItem[]>("list_category_items", { input }),
  listCategoryItemsPage: (input: {
    projectId: string;
    category: "unclassified" | "scene" | "private";
    page: number;
    pageSize: number;
  }) => invoke<CaptureItemPage>("list_category_items", { input }),
  listHistory: (input: {
    projectId: string;
    status?: string | null;
    sessionId?: string | null;
    characterId?: string | null;
    limit?: number;
    offset?: number;
    includePrivate?: boolean;
  }) =>
    invoke<CaptureHistoryPage>("list_capture_history", {
      input: {
        projectId: input.projectId,
        sessionId: input.sessionId ?? null,
        characterId: input.characterId ?? null,
        status: input.status ?? null,
        limit: input.limit,
        offset: input.offset,
        includePrivate: input.includePrivate,
      },
    }),
  runtimeStatus: () => invoke<CaptureRuntimeStatus>("get_capture_runtime_status"),
  listDebugLogs: (input: {
    since?: string | null;
    until?: string | null;
    levels?: LogLevel[];
    module?: string | null;
    event?: string | null;
    correlationId?: string | null;
    outcome?: string | null;
    offset?: number;
    limit?: number;
  }) => invoke<LogQueryResult>("list_debug_logs", {
    input: {
      since: input.since ?? null,
      until: input.until ?? null,
      levels: input.levels ?? [],
      module: input.module ?? null,
      event: input.event ?? null,
      correlationId: input.correlationId ?? null,
      outcome: input.outcome ?? null,
      offset: input.offset ?? 0,
      limit: input.limit,
    },
  }),
  recordClientEvent: (input: {
    level: LogLevel;
    module: string;
    event: string;
    message?: string | null;
    operationId?: string | null;
    durationMs?: number | null;
    outcome?: string | null;
    errorCode?: string | null;
  }) => invoke<void>("record_client_event", { input }),
  getLogStatus: () => invoke<LogStatus>("get_log_status"),
  getLogSettings: () => invoke<LogPolicySettings>("get_log_settings"),
  updateLogSettings: (input: LogPolicySettings) =>
    invoke<LogPolicySettings>("update_log_settings", { input }),
  cleanupDebugLogs: () => invoke<LogCleanupResult>("cleanup_debug_logs"),
  getProcessResourceStatus: () =>
    invoke<ProcessResourceStatus>("get_process_resource_status"),
  getStorageResourceStatus: () =>
    invoke<StorageResourceStatus>("get_storage_resource_status"),
  cleanupResource: (kind: "thumbnail_cache" | "capture_output" | "expired_logs") =>
    invoke<CleanupResourceResult>("cleanup_resource", { input: { kind } }),
  getDatabaseStartupStatus: () =>
    invoke<DatabaseStartupStatus>("get_database_startup_status"),
  preflightDatabase: () => invoke<PreflightReport>("preflight_database"),
  createDatabaseBackup: (destination: string) =>
    invoke<DatabaseBackupResult>("create_database_backup", {
      input: { destination },
    }),
  stageDatabaseRestore: (backupPath: string) =>
    invoke<RestoreRequest>("stage_database_restore", {
      input: { backupPath },
    }),
  rebuildDatabaseIndexes: () =>
    invoke<MaintenanceReport>("rebuild_database_indexes"),
  restartAfterDatabaseRestore: () =>
    invoke<void>("restart_after_database_restore"),
  getDiagnosticSummary: () => invoke<string>("get_diagnostic_summary"),
  getAppSettings: () => invoke<AppSettings>("get_app_settings"),
  updateAppSettings: (settings: AppSettings) =>
    invoke<AppSettings>("update_app_settings", { input: { settings } }),
  getArchiveNamingSettings: () =>
    invoke<ArchiveNamingSettings>("get_archive_naming_settings"),
  updateArchiveNamingSettings: (settings: ArchiveNamingSettings) =>
    invoke<ArchiveNamingSettings>("update_archive_naming_settings", {
      input: { settings },
    }),
  getRecognitionSettings: () =>
    invoke<RecognitionSettings>("get_recognition_settings"),
  updateRecognitionSettings: (settings: RecognitionSettings) =>
    invoke<RecognitionSettings>("update_recognition_settings", {
      input: { settings },
    }),
  getProcessingSettings: () =>
    invoke<ProcessingSettings>("get_processing_settings"),
  updateProcessingSettings: (settings: ProcessingSettings) =>
    invoke<ProcessingSettings>("update_processing_settings", { settings }),
  setRecognitionSuggestion: (input: {
    captureItemId: string;
    suggestedCharacterId: string | null;
    confidence: number | null;
    source: "face_bank" | "vision" | "manual" | null;
  }) => invoke<CaptureItem>("set_recognition_suggestion", { input }),
  reviewRecognitionSuggestion: (input: {
    captureItemId: string;
    decision: "accepted" | "rejected";
  }) => invoke<CaptureItem>("review_recognition_suggestion", { input }),
  acceptRecognitionSuggestion: (captureItemId: string) =>
    invoke<CaptureItem>("accept_recognition_suggestion", { captureItemId }),
  rejectSuggestionAndEnroll: (captureItemId: string) =>
    invoke<CaptureItem>("reject_suggestion_and_enroll", { captureItemId }),
  batchRejectAndEnroll: (projectId: string, characterId: string) =>
    invoke<number>("batch_reject_and_enroll", { projectId, characterId }),
  rebuildFaceBank: (projectId: string) =>
    invoke<FaceBankRebuildSummary>("rebuild_face_bank", { projectId }),
  getFaceBankModelStatus: (projectId: string) =>
    invoke<FaceBankModelStatus>("get_face_bank_model_status", { projectId }),
  getRecognitionDefaults: (modelId: string) =>
    invoke<ResolvedRecognitionProfile>("get_recognition_defaults", {
      modelId,
    }),
  listCharacterCaptureItems: (input: {
    projectId: string;
    characterId: string;
  }) => invoke<CaptureItem[]>("list_character_capture_items", { input }),
  listCharacterCaptureItemsPage: (input: {
    projectId: string;
    characterId: string;
    page: number;
    pageSize: number;
  }) => invoke<CaptureItemPage>("list_character_capture_items", { input }),
  listCharacterFaceSamples: (characterId: string) =>
    invoke<FaceSample[]>("list_character_face_samples", { characterId }),
  setFaceSampleStatus: (
    sampleId: string,
    status: "active" | "revoked",
  ) => invoke<FaceSample>("set_face_sample_status", {
    input: { sampleId, status },
  }),
  setFaceSampleFlagged: (sampleId: string, flagged: boolean) =>
    invoke<FaceSample>("set_face_sample_flagged", {
      input: { sampleId, flagged },
    }),
  verifyCaptureIdentity: (input: {
    captureItemId: string;
    characterId: string;
  }) => invoke<VerificationResult>("verify_capture_identity", { input }),
  suggestForCapture: (captureItemId: string) =>
    invoke<CaptureItem>("suggest_for_capture", { input: { captureItemId } }),
  classifyPopupContext: () => invoke<ClassifyPopupContext>("classify_popup_context"),
  getProjectNote: (projectId: string) =>
    invoke<ProjectNote>("get_project_note", { input: { projectId } }),
  updateProjectNote: (projectId: string, content: string) =>
    invoke<ProjectNote>("update_project_note", { input: { projectId, content } }),
  openProjectNote: (projectId: string) =>
    invoke<OpenProjectNoteResult>("open_project_note", { input: { projectId } }),
  revealProjectNote: (projectId: string) =>
    invoke<OpenProjectNoteResult>("reveal_project_note", { input: { projectId } }),
  getVisionSettings: () => invoke<VisionSettings>("get_vision_settings"),
  updateVisionSettings: (settings: VisionSettings) =>
    invoke<VisionSettings>("update_vision_settings", { input: { settings } }),
  listBundledFonts: () => invoke<BundledFont[]>("list_bundled_fonts"),
  checkVision: () => invoke<VisionHealth>("check_vision_engine"),
  readImage: (captureItemId: string, variant: "source" | "annotated" | "avatar" | "destination") =>
    invoke<ArrayBuffer>("read_capture_image", { input: { captureItemId, variant } }),
  readThumbnail: (captureItemId: string, variant: "source" | "annotated" | "avatar" | "destination") =>
    invoke<ArrayBuffer>("read_capture_thumbnail", { input: { captureItemId, variant } }),
  getThumbnailCacheStatus: () => invoke<ThumbnailCacheStatus>("thumbnail_cache_status"),
};

export async function pickDirectory(): Promise<string | null> {
  const value = await open({ directory: true, multiple: false });
  return typeof value === "string" ? value : null;
}

export async function pickFile(filters?: { name: string; extensions: string[] }[]): Promise<string | null> {
  const value = await open({ directory: false, multiple: false, filters });
  return typeof value === "string" ? value : null;
}

export async function pickSavePath(options: {
  defaultPath?: string;
  filters?: { name: string; extensions: string[] }[];
} = {}): Promise<string | null> {
  const value = await save({ ...options });
  return typeof value === "string" ? value : null;
}

export async function revealPath(path: string): Promise<void> {
  await revealItemInDir(path);
}

// First-party alternative to the opener plugin's scope-gated open_path:
// user-configured directories (custom models/python dirs) can never be listed
// in a static capability scope, so directories are opened via the app command.
export async function openDirectoryExternal(path: string): Promise<void> {
  await invoke("open_directory", { path });
}

export function captureStatusLabel(
  status: CaptureStatus,
  failureStage: "processing" | "archive" | null = null,
): string {
  if (status === "failed" && failureStage === "archive") return "归档失败";
  if (status === "failed" && failureStage === "processing") return "处理失败";
  return {
    awaiting_label: "待分类",
    queued: "排队中",
    processing: "处理中",
    archive_pending: "等待归档",
    completed: "已完成",
    failed: "处理失败",
  }[status];
}

export function captureClassificationLabel(classification: CaptureClassification): string {
  return {
    unclassified: "未分类",
    person: "人物",
    scene: "游戏截图",
    private: "收藏",
  }[classification];
}

export function pathFileName(path: string): string {
  return path.split(/[\\/]/).pop() || path;
}

export function pathJoin(directory: string, name: string): string {
  if (!directory) return name;
  const separator = directory.includes("\\") ? "\\" : "/";
  return `${directory.replace(/[\\/]+$/, "")}${separator}${name}`;
}

export function pathDirectory(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  const index = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  return index > 0 ? trimmed.slice(0, index) : trimmed;
}

export function pathMimeType(path: string): string {
  const suffix = path.toLowerCase().split(".").pop();
  if (suffix === "jpg" || suffix === "jpeg") return "image/jpeg";
  if (suffix === "webp") return "image/webp";
  if (suffix === "bmp") return "image/bmp";
  return "image/png";
}
