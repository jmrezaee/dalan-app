/** A typed folder path that must be resolved before listing: it starts with `~`, or has a `.` or `..` part. */
export const needsResolving = (path: string) => path === '~' || path.startsWith('~/') || /(^|\/)\.\.?(\/|$)/.test(path)
