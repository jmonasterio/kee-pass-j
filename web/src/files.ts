// Sizes and limits for files attached to entries. Every save rewrites the whole database and
// every backup copies it, so large files slow down saving for everything else in the file.

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

/** One file above this gets a warning before it is added. */
export const LARGE_FILE = 5 * 1024 * 1024;
/** All files together above this get a warning. */
export const LARGE_TOTAL = 50 * 1024 * 1024;
/** Full copies the backup folder keeps at most: the newest 20 plus one per day for 30 days. */
export const BACKUP_COPIES = 50;

export const FILE_ADVICE =
  "Good fits: scans of IDs and passports, a will, insurance papers — a few MB each. Photo collections and videos make every save slow and every backup large; keep those elsewhere.";
