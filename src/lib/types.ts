export interface DiskMeta {
  id: number;
  name: string;
  rootPath: string;
  totalSize: number;
  fileCount: number;
  createdAt: string;
}

export interface Node {
  id: number;
  diskId: number;
  parentId: number | null;
  name: string;
  isDir: boolean;
  size: number | null;
  modified: string | null;
  path: string;
}

export interface DiskStats {
  fileCount: number;
  dirCount: number;
  totalSize: number;
  thumbCount: number;
}

export interface ScanProgress {
  diskId: number;
  processed: number;
  current: string;
  done: boolean;
}

export interface SearchHit {
  node: Node;
  diskName: string;
}
