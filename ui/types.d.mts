export type Tone = 'info' | 'success' | 'warning' | 'error';

export type VisualTone = '' | 'warn' | 'bad' | 'good' | 'success' | 'error';

export type DisplayPreference = 'auto' | 'mobile' | 'panel' | 'desktop';

export type Availability = 'ready' | 'stale' | 'not_seen' | 'ambiguous' | 'unavailable';

export type BlindAction = 'open' | 'close' | 'stop' | 'toggle' | 'set_position' | 'set_motor_speed'
  | 'calibration_start' | 'calibration_nudge_open' | 'calibration_set_open_limit'
  | 'calibration_nudge_closed' | 'calibration_set_closed_limit' | 'calibration_stop';

export type CommandStatus =
  | 'queued'
  | 'connecting'
  | 'deferred'
  | 'write_acknowledged'
  | 'outcome_unknown'
  | 'write_failed'
  | 'connection_failed'
  | 'expired'
  | 'cancelled';

export type BatchStatus = 'submitting' | 'checking' | 'outcome_unknown' | 'queued' | 'running'
  | 'completed' | 'partial' | 'failed' | 'rejected';

export type BatchAction = 'open' | 'close';

export interface Position {
  current_closed_percent?: number | null;
  target_closed_percent?: number | null;
}

export interface Enrollment {
  id: string;
  logical_id?: string;
  advertised_name?: string | null;
  label?: string;
  is_simulated?: boolean;
  room?: string;
}

export interface ManagedDevice {
  enrollment: Enrollment;
  availability?: Availability;
  position?: Position | null;
  position_freshness?: string;
  position_observed_at?: string | null;
  last_seen_at?: string | null;
  advertisement_age_ms?: number | null;
  gatt_readiness?: GattReadiness | null;
  motion_feedback?: MotionFeedback | null;
  capabilities?: Record<string, boolean>;
  calibration_mode_active?: boolean;
  battery?: BatteryReading | null;
}

export interface Observation {
  id: string;
  logical_id?: string;
  advertised_name?: string | null;
  device_name?: string | null;
  is_simulated?: boolean;
  freshness?: string;
  last_seen_at?: string | null;
  advertisement_age_ms?: number | null;
  movement_status?: 'opening' | 'closing' | 'at_target' | 'unavailable';
  rssi_dbm?: number | null;
  battery?: BatteryReading | null;
}

export type ObservationIdentity = Pick<Observation, 'logical_id' | 'advertised_name' | 'is_simulated'>
  & Partial<Omit<Observation, 'logical_id' | 'advertised_name' | 'is_simulated'>>;

export interface GattReadiness {
  status?: CommandStatus | 'untested';
  cooldown_until?: string | null;
  transport_evidence?: string;
  last_write_at?: string | null;
  last_attempt_at?: string | null;
  last_write_acknowledged_at?: string | null;
  failure_code?: string | null;
  error_code?: string | null;
  failure_stage?: string | null;
}

export interface MotionFeedback {
  status?: 'awaiting_progress' | 'progress_observed' | 'stalled' | 'target_reached' | 'unavailable';
  estimated_travel_rate_percent_per_second?: number | null;
  [key: string]: unknown;
}

export interface BatteryReading {
  percent_estimate?: number | null;
  status?: string;
  voltage_mv?: number | null;
}

export interface CommandRecord {
  id?: string;
  status: CommandStatus;
  action?: BlindAction;
  accepted_at?: string;
  retryable?: boolean;
  failure_stage?: string;
  [key: string]: unknown;
}

export interface ActivityEntry {
  kind: string;
  subject?: string;
  action?: string;
  status?: string;
  detail?: string;
  occurred_at?: string;
  completed_at?: string;
}

export interface SkippedBlind {
  label?: string;
  availability?: string;
}

export interface BatchTarget {
  label?: string;
  status: CommandStatus;
}

export interface BatchResponse {
  id: string;
  action: BatchAction;
  status: Exclude<BatchStatus, 'submitting' | 'checking' | 'outcome_unknown'>;
  target_count?: number;
  acknowledged_count?: number;
  failed_count?: number;
  pending_count?: number;
  skipped_count?: number;
  skipped?: SkippedBlind[];
  targets?: BatchTarget[];
  error?: string;
  [key: string]: unknown;
}

export interface BatchState {
  id?: string | null;
  action: BatchAction;
  status: BatchStatus;
  idempotencyKey?: string;
  data?: Partial<BatchResponse> | null;
  error?: string | null;
  target_count?: number;
  acknowledged_count?: number;
  failed_count?: number;
  pending_count?: number;
  skipped_count?: number;
  skipped?: SkippedBlind[];
  targets?: BatchTarget[];
}

export interface Diagnostics {
  demo_mode?: boolean;
  simulator_ready?: boolean;
  scanner?: {
    adapter_available?: boolean;
    scan_active?: boolean;
    status?: string;
    soma_advertisements_seen?: number;
    last_error_code?: string | null;
  };
  history_enabled?: boolean;
}

export interface Simulator {
  id: string;
  advertised_name: string;
  current_closed_percent?: number;
  target_closed_percent?: number;
  scenario?: string;
  wireless_profile?: string;
  battery_state?: string;
}

export interface SnackbarAction {
  label: string;
  action: string;
}

export interface Notification {
  message: string;
  tone?: Tone;
  key?: string | null;
  action?: SnackbarAction | null;
  duration?: number;
  details?: string[];
}

export interface Scheduler {
  now(): number;
  setTimeout(callback: () => void, delay: number): unknown;
  clearTimeout(handle: unknown): void;
}

export interface SnackbarItem extends Notification {
  id: string;
  tone: Tone;
  key: string | null;
  action: SnackbarAction | null;
  duration: number;
}

export interface ApiError extends Error {
  status?: number;
  code?: string;
}

export type ApiRequest = (path: string, options?: RequestInit) => Promise<unknown>;

export interface RefreshTarget {
  demoMode?: boolean;
  managed?: ManagedDevice[];
  observations?: Observation[];
  diagnostics?: Diagnostics | null;
  activity?: ActivityEntry[];
  commands?: Record<string, CommandRecord>;
  batch?: BatchState | null;
  serviceError?: string | null;
  confirmingRemovalId?: string | null;
  pendingRemovalFocusId?: string | null;
  removingEnrollmentId?: string | null;
  busy?: Set<string>;
  reconciliationRequired?: Set<string>;
  tokens?: Record<string, symbol>;
  simulators?: Simulator[] | null;
  notifications?: Notification[];
  notify?: (item: Notification) => unknown;
  dismissNotification?: (key: string) => unknown;
}

export interface DashboardState extends RefreshTarget {
  demoMode: boolean;
  managed: ManagedDevice[];
  observations: Observation[];
  diagnostics: Diagnostics | null;
  activity: ActivityEntry[];
  commands: Record<string, CommandRecord>;
  batch: BatchState | null;
  serviceError: string | null;
  confirmingRemovalId: string | null;
  pendingRemovalFocusId: string | null;
  removingEnrollmentId: string | null;
  busy: Set<string>;
  reconciliationRequired: Set<string>;
  tokens: Record<string, symbol>;
  simulators: Simulator[] | null;
  notify: (item: Notification) => unknown;
  dismissNotification: (key: string) => unknown;
}
