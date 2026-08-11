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
}

let snapshot: WorkbenchSnapshot | null = null;

export function readWorkbenchSnapshot(projectId: string | null): WorkbenchSnapshot | null {
  if (!snapshot || (projectId && snapshot.projectId !== projectId)) return null;
  return snapshot;
}

export function writeWorkbenchSnapshot(value: WorkbenchSnapshot): void {
  snapshot = {
    ...value,
    projects: [...value.projects],
    summaries: [...value.summaries],
    characters: [...value.characters],
    items: [...value.items],
    samples: [...value.samples],
  };
}

export function resetWorkbenchSnapshot(): void {
  snapshot = null;
}
