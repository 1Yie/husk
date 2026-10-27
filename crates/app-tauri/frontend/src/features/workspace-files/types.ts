export interface WorkspaceEntry {
  name: string;
  path: string;
  kind: "directory" | "file" | "symlink";
  size: number;
}

export interface DirectoryListing {
  path: string;
  entries: WorkspaceEntry[];
  truncated: boolean;
}

export interface FilePreview {
  path: string;
  name: string;
  size: number;
  kind: "text" | "image" | "binary" | "too_large";
  text?: string;
  mime?: string;
  data?: string;
}
