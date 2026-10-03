// Folder paths the user types or pastes into the copy dialog.

/** `text` as a folder path: outer spaces go, and so do the quotes that Explorer's
 *  "Copy as path" puts around a path. */
export function folderPath(text: string): string {
  const trimmed = text.trim();
  const quoted = trimmed.length >= 2 && trimmed.startsWith('"') && trimmed.endsWith('"');
  return quoted ? trimmed.slice(1, -1).trim() : trimmed;
}
