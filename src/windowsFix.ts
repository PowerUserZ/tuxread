// The PowerShell commands that make Windows mount a partition it skips. The window shows them
// and the user runs them: TuxRead itself never writes to a disk (spec §5.6).
import type { FixView } from "./api";

/** `text` as a single-quoted PowerShell string. */
const quote = (text: string) => `'${text.replaceAll("'", "''")}'`;

/** Finds the disk by the number TuxRead read it as and its own id, and the partition by its
 *  offset and current type, so the commands change nothing when any of them has changed; then
 *  sets the type Windows mounts, and stops there if Windows refuses. */
export function fixScript(fix: FixView, notFound: string): string {
  const gpt = fix.table === "gpt";
  const disk = gpt ? `Guid -eq ${quote(fix.disk)}` : `Signature -eq ${fix.disk}`;
  const type = gpt ? "GptType" : "MbrType";
  const value = (v: string) => (gpt ? quote(v) : v);
  return [
    `$p = Get-Disk -Number ${fix.number} | Where-Object ${disk} | Get-Partition | Where-Object { $_.Offset -eq ${fix.offset} -and $_.${type} -eq ${value(fix.from)} }`,
    "if ($p) {",
    `    $p | Set-Partition -${type} ${value(fix.to)} -ErrorAction Stop`,
    "    $p = $p | Get-Partition",
    "    if (-not [char]::IsLetter($p.DriveLetter)) { $p | Add-PartitionAccessPath -AssignDriveLetter }",
    "    $p | Get-Partition | Get-Volume | Format-List DriveLetter, FileSystemType, FileSystemLabel",
    "} else {",
    `    ${quote(notFound)}`,
    "}",
  ].join("\n");
}
