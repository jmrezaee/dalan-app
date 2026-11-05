// Host icon from the OS name Rust detected (`hosts.os_hint`). A letter badge, not a logo:
// no brand artwork is bundled. Unknown or not yet detected shows the generic "S".
const NAMES: Record<string, string> = {
  macos: 'macOS', ubuntu: 'Ubuntu', debian: 'Debian', fedora: 'Fedora', centos: 'CentOS', rhel: 'Red Hat',
  rocky: 'Rocky Linux', almalinux: 'AlmaLinux', arch: 'Arch Linux', alpine: 'Alpine', opensuse: 'openSUSE',
  linux: 'Linux', freebsd: 'FreeBSD', openbsd: 'OpenBSD', netbsd: 'NetBSD', windows: 'Windows', sunos: 'Solaris',
}

/** One letter for the badge, or `undefined` when nothing is known. */
export function osLetter(hint: string | null | undefined): string | undefined {
  if (!hint || hint === 'unknown') return undefined
  return (NAMES[hint] ?? hint).charAt(0).toUpperCase()
}

/** A readable name for the badge's tooltip. */
export function osName(hint: string | null | undefined): string | undefined {
  if (!hint || hint === 'unknown') return undefined
  return NAMES[hint] ?? hint.charAt(0).toUpperCase() + hint.slice(1)
}
