export function fmtSize(bytes: number | null | undefined): string {
  if (bytes == null || Number.isNaN(bytes)) return "";
  if (bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let v = bytes;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(v >= 100 || i === 0 ? 0 : 1)} ${units[i]}`;
}

export function fmtDate(iso: string | null | undefined): string {
  if (!iso) return "";
  const d = new Date(iso);
  return Number.isNaN(d.getTime())
    ? iso
    : d.toLocaleString("es-ES", { dateStyle: "short", timeStyle: "short" });
}

export function fmtCount(n: number): string {
  return n.toLocaleString("es-ES");
}
