/**
 * Per-file / per-function "interestingness" score for the Change
 * Analysis dashboard's `LookHereFirstCard` and supporting panels.
 *
 * The score is local to the diff — no git history, no blame, no
 * coverage. We combine static-metric spikes multiplicatively in
 * the CRAP-metric tradition: one hot factor (e.g. a +14 complexity
 * spike) should dominate, and additive `1 +` on each factor keeps
 * zeros from collapsing the whole score. Every factor that
 * contributes ≥ 1.2 also produces a human-readable `reason`
 * string so the UI can show "why did this rank high?" on hover.
 *
 * Tunables live in `INTERESTINGNESS_WEIGHTS` so we can retune in
 * one place after dogfooding.
 */
import type { BranchChangeEntry } from "../../api-types.js";
import type { FunctionsBuckets } from "./analysisHelpers.js";

export const INTERESTINGNESS_WEIGHTS = {
  /** Hidden floor on file size factor — keeps zero-line files from
   *  collapsing the score to zero. */
  baseFloor: 1.0,
  /** Multiplier coefficient on positive complexity spike. */
  complexityCoeff: 0.6,
  /** Multiplier coefficient on positive parameter-count spike. */
  paramCoeff: 0.4,
  /** Lines threshold above which a *new* function counts as
   *  "long" — every additional line above the threshold scaled
   *  by `longNewFnDivisor` adds to the multiplier. */
  longNewFnThreshold: 60,
  longNewFnDivisor: 40,
  /** Multiplier applied when no test file changed in the same
   *  top-level dir as this file. */
  untestedMultiplier: 1.5,
  /** Reason threshold — only factors that push the multiplier
   *  this high get an entry in `reasons`. */
  reasonThreshold: 1.2,
} as const;

export interface InterestingnessResult {
  score: number;
  reasons: string[];
}

export interface FileInterestInputs {
  file: BranchChangeEntry;
  /** Functions on this file in the head version that ARE in any of
   *  the four buckets. Caller pre-filters by path. */
  bucketed: {
    added: FunctionsBuckets["added"];
    deleted: FunctionsBuckets["deleted"];
    modifiedSignature: FunctionsBuckets["modifiedSignature"];
    modifiedBody: FunctionsBuckets["modifiedBody"];
  };
  /** True iff a test file changed in the same top-level dir. */
  hasMatchingTest: boolean;
}

export function fileInterestingness(
  input: FileInterestInputs,
): InterestingnessResult {
  const { file, bucketed, hasMatchingTest } = input;
  const W = INTERESTINGNESS_WEIGHTS;
  const adds = file.additions ?? 0;
  const dels = file.deletions ?? 0;
  const sizeFactor = Math.log2(1 + adds + dels);

  const complexitySpike = bucketed.modifiedBody
    .filter((fn) => fn.complexityDelta > 0)
    .reduce((acc, fn) => acc + fn.complexityDelta, 0);

  const paramSpike = bucketed.modifiedSignature
    .filter((fn) => fn.after - fn.before > 0)
    .reduce((acc, fn) => acc + (fn.after - fn.before), 0);

  const longestNewFnExcess = bucketed.added
    .filter((fn) => "length" in fn && (fn as { length: number }).length > W.longNewFnThreshold)
    .reduce(
      (max, fn) =>
        Math.max(max, ((fn as unknown as { length?: number }).length ?? 0) - W.longNewFnThreshold),
      0,
    );
  const longNewFn = Math.max(0, longestNewFnExcess) / W.longNewFnDivisor;

  const untestedMul = hasMatchingTest ? 1.0 : W.untestedMultiplier;

  const base = W.baseFloor + sizeFactor;
  const complexityFactor = 1 + W.complexityCoeff * complexitySpike;
  const paramFactor = 1 + W.paramCoeff * paramSpike;
  const longFactor = 1 + longNewFn;
  const score = base * complexityFactor * paramFactor * longFactor * untestedMul;

  const reasons: string[] = [];
  if (complexityFactor >= W.reasonThreshold) {
    const fnCount = bucketed.modifiedBody.filter((fn) => fn.complexityDelta > 0).length;
    reasons.push(`complexity +${complexitySpike} across ${fnCount} fn${fnCount === 1 ? "" : "s"}`);
  }
  if (paramFactor >= W.reasonThreshold) {
    const fnCount = bucketed.modifiedSignature.filter((fn) => fn.after - fn.before > 0).length;
    reasons.push(`+${paramSpike} param${paramSpike === 1 ? "" : "s"} across ${fnCount} fn${fnCount === 1 ? "" : "s"}`);
  }
  if (longFactor >= W.reasonThreshold) {
    const longest = Math.round(W.longNewFnThreshold + longestNewFnExcess);
    reasons.push(`added ${longest}-line function`);
  }
  if (untestedMul >= W.reasonThreshold) {
    reasons.push("no test in same dir");
  }
  if (sizeFactor >= 5) {
    reasons.push(`${adds + dels} lines touched`);
  }

  return { score, reasons };
}

export interface FunctionInterestInputs {
  /** From `modifiedBody` or `added`. */
  fn: {
    complexity?: number;
    complexityDelta?: number;
    lengthDelta?: number;
    length?: number;
    paramCount?: number;
  };
  /** Per-function churn row (added/deleted/modified) if available. */
  churn: { addedLines: number; deletedLines: number; modifiedLines: number } | null;
  hasMatchingTest: boolean;
}

export function functionInterestingness(
  input: FunctionInterestInputs,
): InterestingnessResult {
  const { fn, churn, hasMatchingTest } = input;
  const W = INTERESTINGNESS_WEIGHTS;
  const churnLines = (churn?.addedLines ?? 0) + (churn?.deletedLines ?? 0);
  const sizeFactor = Math.log2(1 + churnLines + Math.abs(fn.lengthDelta ?? 0));
  const complexityFactor = 1 + W.complexityCoeff * Math.max(0, fn.complexityDelta ?? 0);
  const longFactor =
    fn.length != null && fn.length > W.longNewFnThreshold
      ? 1 + (fn.length - W.longNewFnThreshold) / W.longNewFnDivisor
      : 1;
  const untestedMul = hasMatchingTest ? 1.0 : W.untestedMultiplier;
  const score = (W.baseFloor + sizeFactor) * complexityFactor * longFactor * untestedMul;
  const reasons: string[] = [];
  if (complexityFactor >= W.reasonThreshold) {
    reasons.push(`complexity +${fn.complexityDelta}`);
  }
  if (longFactor >= W.reasonThreshold) {
    reasons.push(`${fn.length} lines`);
  }
  if (untestedMul >= W.reasonThreshold) {
    reasons.push("no test in same dir");
  }
  return { score, reasons };
}
