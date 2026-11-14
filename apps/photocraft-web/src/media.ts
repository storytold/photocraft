/**
 * PhotoCraft Media Module - Image Generation Provider Routing
 * 
 * Integrates image generation capabilities from open-design's imagesrouter
 * into PhotoCraft's web application. Provides MediaSurface types, ProviderId,
 * and execution policy similar to open-design's media contract.
 * 
 * Reference: open-design packages/contracts/src/api/media.ts
 *            open-design apps/daemon/src/routes/media.ts
 */

// Media surfaces (types of media that can be generated)
export const MEDIA_SURFACES = ['image', 'video', 'audio'] as const;
export type MediaSurface = (typeof MEDIA_SURFACES)[number];

// Media execution modes
export const MEDIA_EXECUTION_MODES = ['enabled', 'disabled'] as const;
export type MediaExecutionMode = (typeof MEDIA_EXECUTION_MODES)[number];

/**
 * Media execution policy - controls what surfaces/models are allowed
 * Similar to open-design's MediaExecutionPolicy
 */
export interface MediaExecutionPolicy {
  mode: MediaExecutionMode;
  allowedSurfaces?: MediaSurface[];
  allowedModels?: string[];
}

export const DEFAULT_MEDIA_EXECUTION_POLICY: MediaExecutionPolicy = {
  mode: 'enabled',
};

/**
 * Media policy target - specifies which surface/model to check
 */
export interface MediaPolicyTarget {
  surface: MediaSurface;
  model?: string;
}

/**
 * Media policy denial - result of policy check
 */
export interface MediaPolicyDenial {
  code: string;
  message: string;
}

/**
 * Check if media execution is denied for a given target
 * Similar to open-design's mediaExecutionPolicyDenial()
 */
export function mediaExecutionPolicyDenial(
  policy: MediaExecutionPolicy,
  target: MediaPolicyTarget
): MediaPolicyDenation | null {
  if (policy.mode === 'disabled') {
    return {
      code: 'MEDIA_EXECUTION_DISABLED',
      message: 'media generation is disabled for this run',
    };
  }
  if (
    Array.isArray(policy.allowedSurfaces) &&
    policy.allowedSurfaces.length > 0 &&
    !policy.allowedSurfaces.includes(target.surface)
  ) {
    return {
      code: 'MEDIA_SURFACE_DENIED',
      message: `media surface "${target.surface}" is not allowed for this run`,
    };
  }
  if (
    target.model &&
    Array.isArray(policy.allowedModels) &&
    policy.allowedModels.length > 0 &&
    !policy.allowedModels.includes(target.model)
  ) {
    return {
      code: 'MEDIA_MODEL_DENIED',
      message: `media model "${target.model}" is not allowed for this run`,
    };
  }
  return null;
}

/** Media policy denial - result of policy check */
export type MediaPolicyDenial = MediaPolicyDenial;

/**
 * Media failure next steps - what to do when generation fails
 * Similar to open-design's MEDIA_FAILURE_NEXT_STEPS
 */
export const MEDIA_FAILURE_NEXT_STEPS = [
  'revise-request',
  'switch-model',
  'open-settings',
  'sign-in',
  'add-credit',
  'retry-later',
  'update-app',
  'unsupported',
  'contact-support',
] as const;

export type MediaFailureNextStep = (typeof MEDIA_FAILURE_NEXT_STEPS)[number];

/**
 * Provider ID - identifies which image generation provider to use
 * Similar to open-design's MediaProviderId which includes 'imagerouter'
 */
export type MediaProviderId =
  | 'openai'
  | 'anthropic'
  | 'google'
  | 'ollama'
  | 'midjourney'
  | 'stable-diffusion'
  | 'imagerouter'
  | 'custom';

/**
 * Media task status
 */
export const MEDIA_TASK_STATUSES = [
  'queued',
  'running',
  'done',
  'failed',
  'interrupted',
] as const;

export type MediaTaskStatus = (typeof MEDIA_TASK_STATUSES)[number];

/**
 * Project media task file metadata
 */
export interface ProjectMediaTaskFile {
  name: string;
  size?: number;
  mtime?: number;
  mime?: string;
  kind?: string;
}

/**
 * Media task error - classified for user action
 */
export interface ProjectMediaTaskError {
  message: string;
  status?: number;
  code?: string;
  subject?: 'prompt' | 'input_image' | 'output_image';
  retryable?: boolean;
  nextStep?: MediaFailureNextStep;
}

/**
 * Media failure signal from provider
 */
export interface MediaFailureSignal {
  code?: string | null | undefined;
  status?: number | null | undefined;
  message?: string | null | undefined;
  retryable?: boolean | null | undefined;
  model?: string | null | undefined;
}

/**
 * Run media task failure - host-observed failure
 */
export interface RunMediaTaskFailure {
  taskId: string;
  surface?: string;
  model?: string;
  failedAt: number;
  error: ProjectMediaTaskError;
}

/**
 * HyperFrames scaffold request for local generation
 */
export interface HyperFramesScaffoldRequest {
  compositionDir: string;
}

/**
 * HyperFrames scaffold response - files created locally
 */
export interface HyperFramesScaffoldResponse {
  compositionDir: string;
  files: ['hyperframes.json', 'meta.json', 'index.html', ...string[]];
}

/**
 * Project media task - consumed by UI progress rows
 */
export interface ProjectMediaTask {
  taskId: string;
  sequence?: number;
  batchId?: string;
  batchIndex?: number;
  batchSize?: number;
  runId?: string;
  status: MediaTaskStatus;
  startedAt: number;
  endedAt: number | null;
  elapsed: number;
  surface?: string;
  model?: string;
  progress: string[];
  progressCount: number;
  file?: ProjectMediaTaskFile;
  error?: ProjectMediaTaskError;
}

/**
 * Response containing multiple media tasks
 */
export interface ProjectMediaTasksResponse {
  tasks: ProjectMediaTask[];
}

/**
 * Supported image model IDs (placeholder - will be populated per-provider)
 */
export const SUPPORTED_IMAGE_MODELS: Record<MediaProviderId, string[]> = {
  openai: ['dall-e-3', 'dall-e-2'],
  anthropic: ['claude-3-opus-image', 'claude-3-sonnet-image'],
  google: ['imagen-3'],
  ollama: ['llama3'],
  midjourney: ['v6', 'v5'],
  'stable-diffusion': ['1.5', '2.1'],
  imagerouter: ['auto'],
  custom: [],
};

/**
 * Default provider for image generation when none specified
 */
export const DEFAULT_MEDIA_PROVIDER: MediaProviderId = 'imagerouter';

/**
 * Get default allowed surfaces for a provider
 */
export function getProviderAllowedSurfaces(provider: MediaProviderId): MediaSurface[] {
  return ['image']; // All providers support image generation by default
}

/**
 * Check if a provider supports a given surface
 */
export function providerSupportsSurface(provider: MediaProviderId, surface: MediaSurface): boolean {
  const allowed = getProviderAllowedSurfaces(provider);
  return allowed.includes(surface);
}

export default {
  MEDIA_SURFACES,
  MEDIA_EXECUTION_MODES,
  MediaExecutionMode,
  MediaExecutionPolicy,
  DEFAULT_MEDIA_EXECUTION_POLICY,
  MediaPolicyTarget,
  mediaExecutionPolicyDenial,
  MEDIA_FAILURE_NEXT_STEPS,
  MediaFailureNextStep,
  MediaProviderId,
  MEDIA_TASK_STATUSES,
  ProjectMediaTaskFile,
  ProjectMediaTaskError,
  MediaFailureSignal,
  RunMediaTaskFailure,
  HyperFramesScaffoldRequest,
  HyperFramesScaffoldResponse,
  ProjectMediaTask,
  ProjectMediaTasksResponse,
  SUPPORTED_IMAGE_MODELS,
  DEFAULT_MEDIA_PROVIDER,
  getProviderAllowedSurfaces,
  providerSupportsSurface,
};