import type { Backend, ModelChoice, Snapshot } from "@/ipc/contract";

export const HEAVY_EFFORTS = ["xhigh", "max", "ultra"];

export const findChoice = (choices: ModelChoice[] | null, model: string | null) => (model ? choices?.find((choice) => choice.value === model || choice.resolved === model) : undefined);

export function effortsFor(snapshot: Snapshot, backend: Backend, model: string | null): string[] {
  return findChoice(snapshot.catalog[backend], model ?? snapshot.defaultModels[backend])?.efforts ?? [];
}

export function unavailableModel(snapshot: Snapshot): string | null {
  const backend = snapshot.settings.backend;
  const model = snapshot.settings.models[backend];
  const choices = snapshot.catalog[backend];
  if (!model || !choices) return null;
  return findChoice(choices, model) ? null : model;
}
