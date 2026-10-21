import { Channel, invoke as tauriInvoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

async function invoke<T>(
  command: string,
  args?: Record<string, unknown>,
  options: { recordFailure?: boolean } = {},
): Promise<T> {
  try {
    return await tauriInvoke<T>(command, args);
  } catch (error) {
    if (command !== "record_client_event" && options.recordFailure !== false) {
      void tauriInvoke("record_client_event", {
        input: {
          level: "error",
          module: "ipc",
          event: "command_failed",
          message: `${command} failed: ${error instanceof Error ? error.message : String(error)}`,
          operationId: command,
          outcome: "failed",
          errorCode: "tauri_invoke_error",
        },
      }).catch(() => undefined);
    }
    throw error;
  }
}

/**
 * Image reads dominate capture-grid browsing. One debug record per successful
 * read lets scripts/analyze-image-timing.ps1 compare the frontend-observed
 * latency (invoke round trip, including IPC transfer) with the backend
 * segments. Best effort only.
 */
function reportImageReadTiming(
  command: string,
  captureItemId: string,
  variant: string,
  bytes: number,
  durationMs: number,
  outcome: "ok" | "failed" = "ok",
): void {
  void tauriInvoke("record_client_event", {
    input: {
      level: "debug",
      module: "capture.image",
      event: "read_timing",
      message: `${command} variant=${variant} bytes=${bytes}`,
      operationId: command,
      captureItemId,
      durationMs,
      outcome,
      channel: "perf",
    },
  }).catch(() => undefined);
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
  /** Recorded source sizes; home uses the separate archive directory scan. */
  totalBytes: number;
  engine: string | null;
  /** Distinct engine versions of this project's installs (e.g. "8.1.3,7");
   *  falls back to the python-major equivalent when unknown. */
  engineVersions: string | null;
  archiveOutputSize: string;
  /** Effective archive format after inheritance (keep/png/jpeg/webp). */
  archiveOutputFormat: string;
  /** Effective archive JPEG quality after inheritance. */
  archiveOutputQuality: number;
  lastActivityAt: string;
  latestCaptureItemId: string | null;
  coverCaptureItemId: string | null;
}

export interface ProjectArchiveUsage {
  projectId: string;
  state: "ready" | "unconfigured" | "unavailable" | "missing" | "partial" | "error";
  mainImageCount: number | null;
  avatarImageCount: number | null;
  totalBytes: number | null;
  scannedAt: string;
  message: string | null;
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

export interface CaptureDeletionPreview {
  captureCount: number;
  destinationFileCount: number;
  networkDestinationFileCount: number;
  localDerivedFileCount: number;
  sourceFilesPreserved: number;
}

export interface FileRecycleFailure {
  path: string;
  error: string;
}

export interface CaptureDeletionResult {
  completed: boolean;
  recordsDeleted: number;
  deletedCaptureItemIds: string[];
  destinationFilesRecycled: number;
  destinationFilesPermanentlyDeleted: number;
  destinationFilesAlreadyMissing: number;
  failures: FileRecycleFailure[];
}

export interface CaptureFileReconcileResult {
  discoveredCount: number;
  sourceMissingCount: number;
  sourceReplacedCount: number;
  destinationMissingCount: number;
  destinationUnavailableCount: number;
  unstableCount: number;
}

export interface ProjectFileReconcileResult {
  sourceScannedDirectoryCount: number;
  sourceScannedFileCount: number;
  sourceCheckedCount: number;
  sourceRelocatedCount: number;
  sourceMissingCount: number;
  sourceReplacedCount: number;
  sourceAmbiguousCount: number;
  sourceUnavailableDirectoryCount: number;
  destinationMissingCount: number;
  destinationUnavailableCount: number;
  destinationRelocatedCount: number;
  destinationAmbiguousCount: number;
  destinationContentMismatchCount: number;
  destinationScannedFileCount: number;
}

export interface RebuildArchiveManifestResult {
  projectId: string;
  manifestCount: number;
  entryCount: number;
  characterCount: number;
  writtenCount: number;
  unchangedCount: number;
  skippedCount: number;
  failedCount: number;
  message: string;
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
  /** Capture behind the chosen representative avatar, when one exists. */
  avatarCaptureItemId?: string | null;
  /** Newest person capture; used as the avatar fallback. */
  latestCaptureItemId?: string | null;
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
  roi: RoiSettings | null;
  bigFace: BigFaceSettings | null;
  /** False skips drawing the character name onto a labeled copy. */
  annotatePerson: boolean | null;
}

export interface RoiSettings {
  /** Extra margin (fraction of the frame) used by the retry detection pass. */
  expandRatio: number | null;
  /** "error" | "largest" | "sharpest" */
  multipleFaces: string | null;
}

export interface BigFaceSettings {
  /** False disables the automatic downscaled detection pass. */
  enabled: boolean | null;
  /** Images with a shorter long side never run the extra pass. */
  minImageSide: number | null;
  /** The downscaled pass only wins for faces at least this large in pixels. */
  minFaceSize: number | null;
}

/** A suspicious stored face box that the batch repair will probe. */
export interface FaceRepairCandidate {
  missingFaceBox: boolean;
  id: string;
  sourcePath: string;
  fileName: string;
  storedWidth: number;
  storedHeight: number;
  storedConfidence: number;
}

/** A capture whose reprocess could not replace its previous archive. */
export interface FaceRepairRetry {
  id: string;
  fileName: string;
}

export interface FaceRepairPreview {
  /** Low-confidence or missing boxes that need an engine probe. */
  candidates: FaceRepairCandidate[];
  /** Stuck archive replacements that are re-queued directly. */
  retries: FaceRepairRetry[];
}

export interface FaceRepairStatus {
  state: "idle" | "running" | "done" | "partial" | "failed";
  projectId: string | null;
  taskId: string | null;
  total: number;
  processed: number;
  requeued: number;
  kept: number;
  failed: number;
  currentFile: string | null;
  message: string | null;
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

export type CaptureSourceFileState =
  | "unknown"
  | "available"
  | "missing"
  | "replaced";

export type CaptureDestinationFileState =
  | "none"
  | "unknown"
  | "available"
  | "missing"
  | "unavailable";

export interface CaptureItem {
  processingVersion?: number;
  manualFaceRoiJson?: string | null;
  manualFaceRoiReady?: number;
  /** `auto` (default) or `normalized` (整脸优化) for this capture. */
  faceDetectionMode?: string;
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
  /** Availability of the original screenshot; `replaced` means the path now holds different content. */
  sourceFileState?: CaptureSourceFileState;
  destinationFileState?: CaptureDestinationFileState;
  destinationAvatarFileState?: CaptureDestinationFileState;
}

export interface FaceRoi { x: number; y: number; width: number; height: number }
export interface CaptureResetInput {
  projectId: string;
  captureItemIds?: string[];
  sessionId?: string | null;
  characterId?: string | null;
  status?: string | null;
  includePrivate?: boolean;
}
export interface ResetJob {
  executing: boolean;
  deleteDestinationFiles: boolean | null;
  allowPermanentNetworkDelete: boolean | null;
  id: string;
  status: string;
  items: Array<{
    captureItemId: string;
    sourcePath: string;
    status: string;
    /** Failure class (source_gone, network, locked, denied, changed, busy, unknown). */
    reason: string | null;
    error: string | null;
  }>;
  destinationFileCount: number;
  networkDestinationFileCount: number;
}

export interface CaptureHistoryEntry extends CaptureItem {
  projectId: string;
  projectName: string;
  sessionStatus: string;
  characterName: string | null;
  /** Parsed facts for the original and archived file, when recorded. */
  sourceImage: SourceImageFacts | null;
  destinationImage: ArchiveImageFacts | null;
}

/** Header facts about the original capture file. Unknown parts stay null. */
export interface SourceImageFacts {
  width: number | null;
  height: number | null;
  format: string | null;
}

/** Facts about the archived file: resolution, format, quality and size. */
export interface ArchiveImageFacts {
  width: number | null;
  height: number | null;
  format: string | null;
  bytes: number | null;
  /** `lossless`, `quality` or `unknown`; null when nothing was recorded. */
  qualityKind: "lossless" | "quality" | "unknown" | null;
  /** JPEG quality actually used; null for lossless or unknown outputs. */
  quality: number | null;
  /** Requested size policy: keep / 1080p / 1440p / 2160p / native. */
  sizePolicy: string | null;
  /** False when the requested size policy could not be applied. */
  sizePolicyApplied: boolean | null;
  /** False when a requested format conversion could not be applied. */
  formatApplied: boolean | null;
}

/** Both file variants of one capture, from get_capture_image_facts. */
export interface CaptureImageFacts {
  source: SourceImageFacts | null;
  destination: ArchiveImageFacts | null;
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
  /** Performance instrumentation goes to its own file; off keeps the code. */
  performanceLogging: boolean;
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
  archiveManifestAutoWrite: boolean;
  thumbnailCacheSizeMb: number;
  imageProcessingCoreLimit: number;
  captureOutputAutoCleanupEnabled: boolean;
  captureOutputAutoCleanupThresholdMb: number;
  captureOutputAutoCleanupRetentionDays: number;
}

/** Global default, stored separately from AppSettings. */
export interface RenpyScreenshotSettings {
  autoHideUi: boolean;
}

export interface RenpyScreenshotPolicyView {
  /** null inherits the global default. */
  projectAutoHideUi: boolean | null;
  globalAutoHideUi: boolean;
  resolvedAutoHideUi: boolean;
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
  getCaptureFaceRoi: (captureItemId: string) => invoke<FaceRoi | null>("get_capture_face_roi", { captureItemId }),
  setCaptureFaceRoi: (
    captureItemId: string,
    faceRoi: FaceRoi | null,
    mode?: "auto" | "normalized",
  ) => invoke<CaptureItem>("set_capture_face_roi", { input: { captureItemId, faceRoi, mode } }),
  previewFaceRepair: (projectId: string) =>
    invoke<FaceRepairPreview>("preview_face_repair", { projectId }),
  startFaceRepair: (projectId: string) =>
    invoke<FaceRepairStatus>("start_face_repair", { projectId }),
  getFaceRepairStatus: () => invoke<FaceRepairStatus>("get_face_repair_status"),
  previewCaptureReset: (input: CaptureResetInput) => invoke<ResetJob>("preview_capture_reset", { input }),
  executeCaptureReset: (input: { jobId: string; deleteDestinationFiles: boolean; allowPermanentNetworkDelete: boolean }) =>
    invoke<ResetJob>("execute_capture_reset", { input }),
  getCaptureReset: (jobId: string) => invoke<ResetJob>("get_capture_reset", { jobId }),
  /** Drops an unconfirmed preview so an abandoned dialog cannot pile up. */
  discardCaptureReset: (jobId: string) => invoke<void>("discard_capture_reset", { jobId }),
  listCaptureResetCandidates: (input: CaptureResetInput) => invoke<string[]>("list_capture_reset_candidates", { input }),
  listProjectPendingCaptures: (projectId: string) => invoke<CaptureItem[]>("list_project_pending_captures", { projectId }),
  listProjects: () => invoke<Project[]>("list_projects"),
  listProjectOverviews: () =>
    invoke<ProjectOverviewSummary[]>("list_project_overviews"),
  listProjectArchiveUsage: (
    requestId: string,
    onProgress: (usage: ProjectArchiveUsage) => void,
  ) => {
    const channel = new Channel<ProjectArchiveUsage>();
    channel.onmessage = onProgress;
    return invoke<ProjectArchiveUsage[]>("list_project_archive_usage", {
      requestId,
      onProgress: channel,
    });
  },
  cancelProjectArchiveUsage: (requestId: string) =>
    invoke<void>("cancel_project_archive_usage", { requestId }),
  renameProject: (projectId: string, name: string) =>
    invoke<Project>("rename_project", { input: { projectId, name } }),
  previewProjectDelete: (projectId: string) =>
    invoke<ProjectDeletionPreview>("preview_project_deletion", { projectId }),
  deleteProject: (projectId: string) =>
    invoke<ProjectDeletionPreview>("delete_project", { projectId }),
  previewCaptureDeletion: (captureItemId: string) =>
    invoke<CaptureDeletionPreview>("preview_capture_deletion", {
      input: { captureItemId },
    }),
  deleteCaptureItem: (input: {
    captureItemId: string;
    deleteDestinationFiles: boolean;
    allowPermanentNetworkDelete?: boolean;
  }) => invoke<CaptureDeletionResult>("delete_capture_item", { input }),
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
  previewCharacterDeletion: (input: {
    characterId: string;
    deleteDestinationFiles: boolean;
  }) => invoke<CaptureDeletionPreview>("preview_character_deletion", { input }),
  deleteCharacter: (input: {
    characterId: string;
    deleteDestinationFiles: boolean;
    allowPermanentNetworkDelete?: boolean;
  }) => invoke<CaptureDeletionResult>("delete_character", { input }),
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
  reconcileCaptureFiles: (sessionId: string) =>
    invoke<CaptureFileReconcileResult>("reconcile_capture_files", {
      input: { sessionId },
    }),
  reconcileProjectFiles: (projectId: string, archiveSearchDirectory?: string | null) =>
    invoke<ProjectFileReconcileResult>("reconcile_project_files", {
      input: { projectId, archiveSearchDirectory: archiveSearchDirectory ?? null },
    }),
  rebuildArchiveManifest: (projectId: string) =>
    invoke<RebuildArchiveManifestResult>("rebuild_archive_manifest", {
      input: { projectId },
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
    captureItemId?: string | null;
    durationMs?: number | null;
    outcome?: string | null;
    errorCode?: string | null;
    /** "perf" routes into the performance log; omitted keeps the main log. */
    channel?: "log" | "perf";
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
  getRenpyScreenshotSettings: () =>
    invoke<RenpyScreenshotSettings>("get_renpy_screenshot_settings"),
  updateRenpyScreenshotSettings: (settings: RenpyScreenshotSettings) =>
    invoke<RenpyScreenshotSettings>("update_renpy_screenshot_settings", {
      input: { settings },
    }),
  getArchiveNamingSettings: () =>
    invoke<ArchiveNamingSettings>("get_archive_naming_settings"),
  updateArchiveNamingSettings: (settings: ArchiveNamingSettings) =>
    invoke<ArchiveNamingSettings>("update_archive_naming_settings", {
      input: { settings },
    }),
  getArchiveOutputSettings: () =>
    invoke<ArchiveOutputSettings>("get_archive_output_settings"),
  updateArchiveOutputSettings: (settings: ArchiveOutputSettings) =>
    invoke<ArchiveOutputSettings>("update_archive_output_settings", {
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
  getCaptureImageFacts: (captureItemId: string) =>
    invoke<CaptureImageFacts>("get_capture_image_facts", {
      input: { captureItemId },
    }),
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
  readImage: async (
    captureItemId: string,
    variant: "source" | "annotated" | "avatar" | "destination",
  ) => {
    const started = performance.now();
    try {
      const bytes = await invoke<ArrayBuffer>(
        "read_capture_image",
        { input: { captureItemId, variant } },
        // Media reads are speculative: cards deliberately fall back to another
        // variant or an ImageOff placeholder when files were removed outside
        // Scene Vault. The backend still updates file state, while the caller
        // decides whether an explicit preview should surface the error.
        { recordFailure: false },
      );
      reportImageReadTiming(
        "read_capture_image",
        captureItemId,
        variant,
        bytes.byteLength,
        performance.now() - started,
      );
      return bytes;
    } catch (error) {
      reportImageReadTiming("read_capture_image", captureItemId, variant, 0, performance.now() - started, "failed");
      throw error;
    }
  },
  readThumbnail: async (
    captureItemId: string,
    variant: "source" | "annotated" | "avatar" | "destination",
  ) => {
    const started = performance.now();
    try {
      const bytes = await invoke<ArrayBuffer>(
        "read_capture_thumbnail",
        { input: { captureItemId, variant } },
        { recordFailure: false },
      );
      reportImageReadTiming(
        "read_capture_thumbnail",
        captureItemId,
        variant,
        bytes.byteLength,
        performance.now() - started,
      );
      return bytes;
    } catch (error) {
      reportImageReadTiming("read_capture_thumbnail", captureItemId, variant, 0, performance.now() - started, "failed");
      throw error;
    }
  },
  getThumbnailCacheStatus: () => invoke<ThumbnailCacheStatus>("thumbnail_cache_status"),
};

export interface EngineDetection {
  engine: string;
  engineRoot: string;
  gameDirectory: string;
  pythonMajor: number | null;
  /** Exact engine version from the bundled Ren'Py version files. */
  engineVersion: string | null;
  bridgeInstalled: boolean;
}

export interface ProjectEngineIntegration {
  id: string;
  projectId: string;
  engine: string;
  engineRoot: string;
  gameDirectory: string;
  pythonMajor: number | null;
  engineVersion: string | null;
  bridgeVersion: string;
  /** Install-time fallback kept for compatibility with earlier bridges. */
  autoHideUi: boolean;
  /** null follows the project policy. */
  autoHideUiOverride: boolean | null;
  manifestJson: string;
  installedAt: string;
  updatedAt: string;
}

export interface ProjectEngineIntegrationView {
  integration: ProjectEngineIntegration;
  bridgeFilePresent: boolean;
  compiledFilePresent: boolean;
  bundledBridgeVersion: string;
  resolvedAutoHideUi: boolean;
  /** Install-time advisory, e.g. a kept compiled artifact we cannot verify. */
  warning: string | null;
}

export interface ProjectGameLaunchOption {
  integrationId: string;
  engineRoot: string;
  executable: string;
  gameName: string;
  /** Best guess for the main executable; the menu keeps the alternatives. */
  recommended: boolean;
}

export interface BridgeUninstallResult {
  removedFiles: string[];
  keptFiles: string[];
}

export interface CaptureEngineMetadata {
  captureItemId: string;
  projectId: string;
  engine: string;
  schemaVersion: number;
  bridgeVersion: string;
  /** Engine version reported with this screenshot (bridge 1.1.4+). */
  engineVersion: string | null;
  speakerRaw: string | null;
  speakerSource: string | null;
  dialogue: string | null;
  suggestedCharacterId: string | null;
  virtualWidth: number | null;
  virtualHeight: number | null;
  physicalWidth: number | null;
  physicalHeight: number | null;
  payloadJson: string;
  createdAt: string;
  updatedAt: string;
}

export interface CapturePolicyView {
  projectSize: string;
  resolvedSize: string;
  /** Global default, so the inherit label never shows a project override. */
  globalSize: string;
  projectFormat: string;
  resolvedFormat: string;
  globalFormat: string;
  /** Project JPEG quality; null means "inherit the global default". */
  projectQuality: number | null;
  resolvedQuality: number;
  globalQuality: number;
}

export interface ArchiveOutputSettings {
  size: string;
  format: string;
  quality: number;
}

export const engineApi = {
  detectEngine: (path: string) =>
    invoke<EngineDetection>("detect_engine", { input: { path } }),
  getProjectIntegration: (projectId: string) =>
    invoke<ProjectEngineIntegrationView[]>("get_project_engine_integration", {
      input: { projectId },
    }),
  installBridge: (projectId: string, path: string, autoHideUi?: boolean) =>
    invoke<ProjectEngineIntegrationView>("install_project_engine_bridge", {
      input: { projectId, path, ...(autoHideUi === undefined ? {} : { autoHideUi }) },
    }),
  uninstallBridge: (projectId: string, integrationId?: string) =>
    invoke<BridgeUninstallResult>("uninstall_project_engine_bridge", {
      input: integrationId ? { projectId, integrationId } : { projectId },
    }),
  getScreenshotPolicy: (projectId: string) =>
    invoke<RenpyScreenshotPolicyView>("get_project_renpy_screenshot_policy", {
      input: { projectId },
    }),
  setProjectScreenshotPolicy: (projectId: string, autoHideUi: boolean | null) =>
    invoke<RenpyScreenshotPolicyView>("set_project_renpy_screenshot_policy", {
      input: { projectId, autoHideUi },
    }),
  setInstallationScreenshotPolicy: (
    projectId: string,
    integrationId: string,
    autoHideUi: boolean | null,
  ) => invoke<ProjectEngineIntegrationView>("set_installation_renpy_screenshot_policy", {
    input: { projectId, integrationId, autoHideUi },
  }),
  getCaptureMetadata: (captureItemId: string) =>
    invoke<CaptureEngineMetadata | null>("get_capture_engine_metadata", {
      input: { captureItemId },
    }),
  getCapturePolicy: (projectId: string) =>
    invoke<CapturePolicyView>("get_project_capture_policy", { input: { projectId } }),
  setArchiveOutputSize: (projectId: string, archiveOutputSize: string) =>
    invoke<CapturePolicyView>("set_project_archive_output_size", {
      input: { projectId, archiveOutputSize },
    }),
  setProjectCapturePolicy: (input: {
    projectId: string;
    archiveOutputSize: string;
    archiveOutputFormat: string;
    archiveOutputQuality: number | null;
  }) => invoke<CapturePolicyView>("set_project_capture_policy", { input }),
  listProjectGames: (projectId: string) =>
    invoke<ProjectGameLaunchOption[]>("list_project_games", { input: { projectId } }),
  launchGame: (projectId: string, integrationId?: string, executable?: string) =>
    invoke<ProjectGameLaunchOption>("launch_project_game", {
      input: {
        projectId,
        ...(integrationId ? { integrationId } : {}),
        ...(executable ? { executable } : {}),
      },
    }),
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

/** Reject late responses from an older reset/ROI generation or older row snapshot. */
export function isOlderCapture(incoming: CaptureItem, current: CaptureItem): boolean {
  const next = incoming.processingVersion ?? 0;
  const prior = current.processingVersion ?? 0;
  if (next !== prior) return next < prior;
  return Date.parse(incoming.updatedAt) < Date.parse(current.updatedAt);
}
