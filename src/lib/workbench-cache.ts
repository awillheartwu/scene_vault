import type {
  CaptureItem,
  Character,
  CharacterSummary,
  FaceBankModelStatus,
  FaceSample,
  Project,
} from "@/lib/capture-api";

export interface WorkbenchSnapshot {
  projectId: string;
  projects: Project[];
  summaries: CharacterSummary[];
  characters: Character[];
  modelStatus: FaceBankModelStatus | null;
  selectedCharacterId: string | null;
  items: CaptureItem[];
  samples: FaceSample[];
  selectedItemId: string | null;
  showPrivate: boolean;
  itemPage: number;
  itemPageSize: number;
  itemTotal: number;
}

const snapshots = new Map<string, WorkbenchSnapshot>();

export function readWorkbenchSnapshot(projectId: string | null): WorkbenchSnapshot | null {
  if (!projectId) return null;
  return snapshots.get(projectId) ?? null;
}

export function writeWorkbenchSnapshot(value: WorkbenchSnapshot): void {
  snapshots.set(value.projectId, {
    ...value,
    projects: [...value.projects],
    summaries: [...value.summaries],
    characters: [...value.characters],
    items: [...value.items],
    samples: [...value.samples],
  });
}

export function resetWorkbenchSnapshot(): void {
  snapshots.clear();
}
