/** 0-5 for the S24 strength meter. A rough guide for the user, not a security check (Rust enforces the minimum). */
export function passwordStrength(pw: string): number {
  if (pw.length < 8) return pw.length === 0 ? 0 : 1
  const len = pw.length < 10 ? 1 : pw.length < 12 ? 2 : pw.length < 16 ? 3 : 4
  const classes = [/[a-z]/, /[A-Z]/, /\d/, /[^A-Za-z0-9]/].filter(r => r.test(pw)).length
  const words = pw.split(/[\s\-_.]+/).filter(w => w.length >= 3).length
  const variety = classes >= 3 || words >= 3 ? 1 : 0
  return Math.min(5, len + variety)
}
