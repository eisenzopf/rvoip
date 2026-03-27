const BASE_URL = '/api/v1';

export interface ApiResponse<T> {
  code: number;
  message: string;
  data: T | null;
  request_id: string;
}

// Auth types
export interface LoginRequest { username: string; password: string; }
export interface LoginResponse { access_token: string; refresh_token: string; expires_in: number; user: AuthUser; }
export interface AuthUser { id?: string; user_id?: string; username: string; roles: string[]; }
export interface RefreshRequest { refresh_token: string; }
export interface RefreshResponse { access_token: string; refresh_token: string; expires_in: number; }
export interface ChangePasswordRequest { current_password: string; new_password: string; }

export function getToken(): string | null {
  return localStorage.getItem('rvoip-token');
}

async function request<T>(path: string): Promise<T> {
  const headers: Record<string, string> = {};
  const token = getToken();
  if (token) headers['Authorization'] = `Bearer ${token}`;
  const res = await fetch(`${BASE_URL}${path}`, { headers });
  if (res.status === 401) {
    localStorage.removeItem('rvoip-token');
    localStorage.removeItem('rvoip-refresh-token');
    localStorage.removeItem('rvoip-user');
    window.location.href = '/login';
    throw new Error('Unauthorized');
  }
  const json: ApiResponse<T> = await res.json();
  if (json.code !== 200 || !json.data) throw new Error(json.message);
  return json.data;
}

async function requestWithAuth<T>(path: string): Promise<T> {
  return request<T>(path);
}

// Auth API
export const login = async (data: LoginRequest): Promise<LoginResponse> => {
  const res = await fetch(`${BASE_URL}/auth/login`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(data) });
  if (!res.ok) {
    const err = await res.json().catch(() => ({ message: 'Login failed' }));
    throw new Error(err.message || 'Invalid username or password');
  }
  // Backend returns LoginResponse directly (not wrapped in ApiResponse)
  return res.json();
};

export const refreshToken = (data: RefreshRequest) =>
  fetch(`${BASE_URL}/auth/refresh`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(data) })
    .then(r => r.json()).then((r: ApiResponse<RefreshResponse>) => { if (r.code !== 200 || !r.data) throw new Error(r.message); return r.data; });

export const fetchMe = () => requestWithAuth<AuthUser>('/auth/me');

export const changePassword = (data: ChangePasswordRequest) =>
  fetch(`${BASE_URL}/auth/me/password`, { method: 'PUT', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) })
    .then(r => r.json());

export const logout = () =>
  fetch(`${BASE_URL}/auth/logout`, { method: 'POST', headers: { 'Authorization': `Bearer ${getToken()}` } })
    .then(r => r.json());

// Dashboard
export interface DashboardMetrics {
  active_calls: number;
  active_bridges: number;
  available_agents: number;
  busy_agents: number;
  queued_calls: number;
  total_calls_handled: number;
  sip_registrations: number;
}

export const fetchDashboard = () => request<DashboardMetrics>('/dashboard');

// Activity (24h chart)
export interface HourlyActivity {
  hour: number;
  calls: number;
  queued: number;
}

export interface ActivityResponse {
  hours: HourlyActivity[];
}

export const fetchActivity = () => request<ActivityResponse>('/dashboard/activity');

// Registrations
export interface ContactView {
  uri: string;
  transport: string;
  user_agent: string;
  expires: string;
  q_value: number;
}

export interface RegistrationView {
  user_id: string;
  contacts: ContactView[];
  registered_at: string;
  expires: string;
  capabilities: string[];
}

export interface RegistrationsResponse {
  registrations: RegistrationView[];
  total: number;
}

export const fetchRegistrations = () => request<RegistrationsResponse>('/registrations');

// Calls
export interface ActiveCall {
  call_id: string;
  from_uri: string;
  to_uri: string;
  caller_id: string;
  agent_id: string | null;
  queue_id: string | null;
  status: string;
  priority: number;
  customer_type: string;
  required_skills: string[];
  created_at: string;
  queued_at: string | null;
  answered_at: string | null;
  ended_at: string | null;
}

export const fetchCall = (id: string) => request<ActiveCall>(`/calls/${id}`);

export interface CallsResponse {
  calls: ActiveCall[];
  total: number;
}

export const fetchCalls = () => request<CallsResponse>('/calls');

// Agents
export interface AgentView {
  id: string;
  sip_uri: string;
  contact_uri: string;
  display_name: string;
  status: string;
  skills: string[];
  current_calls: number;
  max_calls: number;
  performance_score: number;
  department?: string;
  extension?: string;
}

export interface AgentsResponse {
  agents: AgentView[];
  total: number;
  online: number;
}

export const fetchAgents = () => request<AgentsResponse>('/agents');

export interface CreateAgentRequest {
  id?: string;
  display_name: string;
  extension?: string;
  sip_uri?: string;
  skills: string[];
  max_concurrent_calls: number;
  department?: string;
}

export const createAgent = (data: CreateAgentRequest) =>
  fetch(`${BASE_URL}/agents`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` },
    body: JSON.stringify(data),
  }).then((r) => r.json());

export const updateAgent = (id: string, data: Partial<CreateAgentRequest>) =>
  fetch(`${BASE_URL}/agents/${id}`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` },
    body: JSON.stringify(data),
  }).then((r) => r.json());

export const updateAgentStatus = (id: string, status: string) =>
  fetch(`${BASE_URL}/agents/${id}/status`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` },
    body: JSON.stringify({ status }),
  }).then((r) => r.json());

export const deleteAgent = (id: string) =>
  fetch(`${BASE_URL}/agents/${id}`, { method: 'DELETE', headers: { 'Authorization': `Bearer ${getToken()}` } }).then((r) => r.json());

// Queues
export interface QueueView {
  queue_id: string;
  total_calls: number;
  avg_wait_secs: number;
  longest_wait_secs: number;
}

export interface QueueConfigView {
  queue_id: string;
  default_max_wait_time: number;
  max_queue_size: number;
  enable_priorities: boolean;
  enable_overflow: boolean;
  announcement_interval: number;
}

export interface QueuesFullResponse {
  queues: QueueView[];
  configs: QueueConfigView[];
  total_waiting: number;
}

export interface QueuedCallView {
  session_id: string;
  from: string;
  to: string;
  status: string;
  priority: number;
  created_at: string;
}

export const fetchQueues = () => request<QueuesFullResponse>('/queues');

export const createQueue = (queue_id: string) =>
  fetch(`${BASE_URL}/queues`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify({ queue_id }) }).then(r => r.json());

export const fetchQueueConfig = (id: string) => request<QueueConfigView>(`/queues/${id}`);

export const updateQueue = (id: string, data: Partial<QueueConfigView>) =>
  fetch(`${BASE_URL}/queues/${id}`, { method: 'PUT', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const deleteQueue = (id: string) =>
  fetch(`${BASE_URL}/queues/${id}`, { method: 'DELETE', headers: { 'Authorization': `Bearer ${getToken()}` } }).then(r => r.json());

export const fetchQueueCalls = (id: string) => request<QueuedCallView[]>(`/queues/${id}/calls`);

export const assignQueueCall = (queueId: string, callId: string, agentId: string) =>
  fetch(`${BASE_URL}/queues/${queueId}/calls/${callId}/assign`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify({ agent_id: agentId }) }).then(r => r.json());

// Users
export interface UserView { id: string; username: string; email: string | null; display_name: string | null; roles: string[]; active: boolean; created_at: string; updated_at: string; last_login: string | null; }
export interface UsersListResponse { users: UserView[]; total: number; }
export interface CreateUserRequest { username: string; password: string; email?: string; display_name?: string; roles: string[]; }
export interface UpdateUserRequest { email?: string; display_name?: string; active?: boolean; }

export const fetchUsers = (params?: { search?: string; role?: string; limit?: number; offset?: number }) => {
  const qs = new URLSearchParams();
  if (params?.search) qs.set('search', params.search);
  if (params?.role) qs.set('role', params.role);
  if (params?.limit) qs.set('limit', String(params.limit));
  if (params?.offset) qs.set('offset', String(params.offset));
  const suffix = qs.toString() ? `?${qs}` : '';
  return request<UsersListResponse>(`/users${suffix}`);
};

export const createUser = (data: CreateUserRequest) =>
  fetch(`${BASE_URL}/users`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const updateUser = (id: string, data: UpdateUserRequest) =>
  fetch(`${BASE_URL}/users/${id}`, { method: 'PUT', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const deleteUser = (id: string) =>
  fetch(`${BASE_URL}/users/${id}`, { method: 'DELETE', headers: { 'Authorization': `Bearer ${getToken()}` } }).then(r => r.json());

export const updateUserRoles = (id: string, roles: string[]) =>
  fetch(`${BASE_URL}/users/${id}/roles`, { method: 'PUT', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify({ roles }) }).then(r => r.json());

// API Keys
export interface ApiKeyView { id: string; name: string; permissions: string[]; expires_at: string | null; last_used: string | null; created_at: string; }
export interface ApiKeyCreatedResponse { key: ApiKeyView; raw_key: string; }

export const fetchApiKeys = (userId: string) => request<ApiKeyView[]>(`/users/${userId}/api-keys`);

export const createApiKey = (userId: string, data: { name: string; permissions: string[] }) =>
  fetch(`${BASE_URL}/users/${userId}/api-keys`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const revokeApiKey = (userId: string, keyId: string) =>
  fetch(`${BASE_URL}/users/${userId}/api-keys/${keyId}`, { method: 'DELETE', headers: { 'Authorization': `Bearer ${getToken()}` } }).then(r => r.json());

// Routing
export interface RoutingConfigView { default_strategy: string; enable_load_balancing: boolean; load_balance_strategy: string; enable_geographic_routing: boolean; enable_time_based_routing: boolean; }
export interface OverflowPolicyView { id: string; name: string; condition_type: string; condition_value: string; action_type: string; action_value: string; priority: number; enabled: boolean; }

export const fetchRoutingConfig = () => request<RoutingConfigView>('/routing/config');
export const updateRoutingConfig = (data: Partial<RoutingConfigView>) =>
  fetch(`${BASE_URL}/routing/config`, { method: 'PUT', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const fetchOverflowPolicies = () => request<OverflowPolicyView[]>('/routing/overflow/policies');
export const createOverflowPolicy = (data: Omit<OverflowPolicyView, 'id'>) =>
  fetch(`${BASE_URL}/routing/overflow/policies`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());
export const updateOverflowPolicy = (id: string, data: Omit<OverflowPolicyView, 'id'>) =>
  fetch(`${BASE_URL}/routing/overflow/policies/${id}`, { method: 'PUT', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());
export const deleteOverflowPolicy = (id: string) =>
  fetch(`${BASE_URL}/routing/overflow/policies/${id}`, { method: 'DELETE', headers: { 'Authorization': `Bearer ${getToken()}` } }).then(r => r.json());

// Health
export interface HealthStatus {
  status: string;
  uptime_secs: number;
  version: string;
}

export const fetchHealth = () => request<HealthStatus>('/system/health');

// Call History
export interface CallHistoryEntry {
  call_id: string;
  customer_number: string | null;
  agent_id: string | null;
  queue_name: string | null;
  start_time: string | null;
  end_time: string | null;
  duration_seconds: number | null;
  disposition: string | null;
  notes: string | null;
}

export const fetchCallHistory = (params?: { limit?: number; offset?: number }) => {
  const qs = new URLSearchParams();
  if (params?.limit) qs.set('limit', String(params.limit));
  if (params?.offset) qs.set('offset', String(params.offset));
  return request<CallHistoryEntry[]>(`/calls/history${qs.toString() ? '?' + qs : ''}`);
};

export const hangupCall = (callId: string) =>
  fetch(`${BASE_URL}/calls/${callId}/hangup`, { method: 'POST', headers: { 'Authorization': `Bearer ${getToken()}` } }).then(r => r.json());

// System Config
export const fetchSystemConfig = () => request<Record<string, unknown>>('/system/config');

export const exportConfig = () => request<string>('/system/config/export');

export const importConfig = (json: string) =>
  fetch(`${BASE_URL}/system/config/import`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify({ config_json: json }) }).then(r => r.json());

// Audit Log
export interface AuditLogEntry {
  id: number;
  user_id: string;
  username: string;
  action: string;
  resource_type: string;
  resource_id: string | null;
  details: unknown;
  created_at: string;
}

export const fetchAuditLog = (params?: { limit?: number; offset?: number }) => {
  const qs = new URLSearchParams();
  if (params?.limit) qs.set('limit', String(params.limit));
  if (params?.offset) qs.set('offset', String(params.offset));
  return request<AuditLogEntry[]>(`/system/audit/log${qs.toString() ? '?' + qs : ''}`);
};

// Presence
export interface PresenceView {
  user_id: string;
  status: string;
  note: string | null;
  last_updated: string;
}

export const fetchPresence = () => request<PresenceView[]>('/presence');

export const updateMyPresence = (status: string, note?: string) =>
  fetch(`${BASE_URL}/presence/me`, { method: 'PUT', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify({ status, note }) }).then(r => r.json());

// Monitoring
export interface RealtimeStats {
  active_calls: number;
  active_bridges: number;
  available_agents: number;
  busy_agents: number;
  queued_calls: number;
  total_calls_handled: number;
  routing_stats: {
    calls_routed_directly: number;
    calls_queued: number;
    calls_rejected: number;
  };
}

export interface AlertView {
  id: string;
  severity: string;
  message: string;
  timestamp: string;
}

export const fetchRealtimeStats = () => request<RealtimeStats>('/monitoring/realtime');

export const fetchAlerts = () => request<AlertView[]>('/monitoring/alerts');

// Departments
export interface DepartmentView {
  id: string;
  name: string;
  parent_id: string | null;
  description: string | null;
  manager_id: string | null;
  agent_count: number;
  created_at: string;
}

export const fetchDepartments = () => request<DepartmentView[]>('/departments');

export const createDepartment = (data: { name: string; description?: string; parent_id?: string }) =>
  fetch(`${BASE_URL}/departments`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const updateDepartment = (id: string, data: { name?: string; description?: string; parent_id?: string }) =>
  fetch(`${BASE_URL}/departments/${id}`, { method: 'PUT', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const deleteDepartment = (id: string) =>
  fetch(`${BASE_URL}/departments/${id}`, { method: 'DELETE', headers: { 'Authorization': `Bearer ${getToken()}` } }).then(r => r.json());

// Extensions
export interface ExtensionRangeView {
  id: string;
  range_start: number;
  range_end: number;
  department_id: string | null;
  description: string | null;
  total: number;
  assigned: number;
  available: number;
}

export interface ExtensionView {
  number: number;
  range_id: string | null;
  agent_id: string | null;
  status: string;
}

export const fetchExtensionRanges = () => request<ExtensionRangeView[]>('/extensions');

export const createExtensionRange = (data: { range_start: number; range_end: number; department_id?: string; description?: string }) =>
  fetch(`${BASE_URL}/extensions/ranges`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const deleteExtensionRange = (id: string) =>
  fetch(`${BASE_URL}/extensions/ranges/${id}`, { method: 'DELETE', headers: { 'Authorization': `Bearer ${getToken()}` } }).then(r => r.json());

export const fetchAvailableExtensions = () => request<ExtensionView[]>('/extensions/available');

export const assignExtension = (number: number, agent_id: string) =>
  fetch(`${BASE_URL}/extensions/${number}/assign`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify({ agent_id }) }).then(r => r.json());

export const releaseExtension = (number: number) =>
  fetch(`${BASE_URL}/extensions/${number}/release`, { method: 'POST', headers: { 'Authorization': `Bearer ${getToken()}` } }).then(r => r.json());

// Skills
export interface SkillView {
  id: string;
  name: string;
  category: string | null;
  description: string | null;
  agent_count: number;
}

export interface AgentSkillView {
  skill_id: string;
  skill_name: string;
  proficiency: number;
}

export const fetchSkills = () => request<SkillView[]>('/skills');

export const createSkill = (data: { name: string; category?: string; description?: string }) =>
  fetch(`${BASE_URL}/skills`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const updateSkill = (id: string, data: { name?: string; category?: string; description?: string }) =>
  fetch(`${BASE_URL}/skills/${id}`, { method: 'PUT', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const deleteSkill = (id: string) =>
  fetch(`${BASE_URL}/skills/${id}`, { method: 'DELETE', headers: { 'Authorization': `Bearer ${getToken()}` } }).then(r => r.json());

export const fetchAgentSkills = (agentId: string) => request<AgentSkillView[]>(`/skills/agents/${agentId}`);

export const setAgentSkills = (agentId: string, skills: { skill_id: string; proficiency: number }[]) =>
  fetch(`${BASE_URL}/skills/agents/${agentId}`, { method: 'PUT', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify({ skills }) }).then(r => r.json());

// Phone Lists (blacklist/whitelist/VIP)
export interface PhoneListEntry {
  id: string;
  number: string;
  list_type: string;
  reason: string | null;
  customer_name: string | null;
  vip_level: number | null;
  expires_at: string | null;
  created_by: string | null;
  created_at: string;
}

export interface PhoneListCheckResult {
  number: string;
  entries: PhoneListEntry[];
}

export interface CreatePhoneListRequest {
  number: string;
  list_type: string;
  reason?: string;
  customer_name?: string;
  vip_level?: number;
  expires_at?: string;
  created_by?: string;
}

export interface UpdatePhoneListRequest {
  number?: string;
  list_type?: string;
  reason?: string;
  customer_name?: string;
  vip_level?: number;
  expires_at?: string;
  created_by?: string;
}

export const fetchPhoneLists = (listType?: string) => {
  const qs = listType ? `?type=${encodeURIComponent(listType)}` : '';
  return request<PhoneListEntry[]>(`/phone-lists${qs}`);
};

export const createPhoneList = (data: CreatePhoneListRequest) =>
  fetch(`${BASE_URL}/phone-lists`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const updatePhoneList = (id: string, data: UpdatePhoneListRequest) =>
  fetch(`${BASE_URL}/phone-lists/${id}`, { method: 'PUT', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const deletePhoneList = (id: string) =>
  fetch(`${BASE_URL}/phone-lists/${id}`, { method: 'DELETE', headers: { 'Authorization': `Bearer ${getToken()}` } }).then(r => r.json());

export const checkPhoneNumber = (number: string) =>
  request<PhoneListCheckResult>(`/phone-lists/check/${encodeURIComponent(number)}`);

// IVR Menus
export interface IvrOptionView {
  id: string;
  digit: string;
  label: string;
  action_type: string;
  action_target: string | null;
  announcement: string | null;
  position: number;
}

export interface IvrMenuView {
  id: string;
  name: string;
  description: string | null;
  welcome_message: string | null;
  timeout_seconds: number;
  max_retries: number;
  timeout_action: string;
  invalid_action: string;
  is_root: boolean;
  business_hours_start: string;
  business_hours_end: string;
  business_days: string;
  after_hours_action: string;
  options: IvrOptionView[];
}

export interface CreateIvrMenuRequest {
  name: string;
  description?: string;
  welcome_message?: string;
  timeout_seconds?: number;
  max_retries?: number;
  timeout_action?: string;
  invalid_action?: string;
  is_root?: boolean;
  business_hours_start?: string;
  business_hours_end?: string;
  business_days?: string;
  after_hours_action?: string;
}

export interface UpdateIvrMenuRequest {
  name?: string;
  description?: string;
  welcome_message?: string;
  timeout_seconds?: number;
  max_retries?: number;
  timeout_action?: string;
  invalid_action?: string;
  is_root?: boolean;
  business_hours_start?: string;
  business_hours_end?: string;
  business_days?: string;
  after_hours_action?: string;
}

export interface CreateIvrOptionRequest {
  digit: string;
  label: string;
  action_type: string;
  action_target?: string;
  announcement?: string;
  position?: number;
}

export interface UpdateIvrOptionRequest {
  digit?: string;
  label?: string;
  action_type?: string;
  action_target?: string;
  announcement?: string;
  position?: number;
}

export const fetchIvrMenus = () => request<IvrMenuView[]>('/ivr');

export const fetchIvrMenu = (id: string) => request<IvrMenuView>(`/ivr/${id}`);

export const createIvrMenu = (data: CreateIvrMenuRequest) =>
  fetch(`${BASE_URL}/ivr`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const updateIvrMenu = (id: string, data: UpdateIvrMenuRequest) =>
  fetch(`${BASE_URL}/ivr/${id}`, { method: 'PUT', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const deleteIvrMenu = (id: string) =>
  fetch(`${BASE_URL}/ivr/${id}`, { method: 'DELETE', headers: { 'Authorization': `Bearer ${getToken()}` } }).then(r => r.json());

export const createIvrOption = (menuId: string, data: CreateIvrOptionRequest) =>
  fetch(`${BASE_URL}/ivr/${menuId}/options`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const updateIvrOption = (menuId: string, optionId: string, data: UpdateIvrOptionRequest) =>
  fetch(`${BASE_URL}/ivr/${menuId}/options/${optionId}`, { method: 'PUT', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const deleteIvrOption = (menuId: string, optionId: string) =>
  fetch(`${BASE_URL}/ivr/${menuId}/options/${optionId}`, { method: 'DELETE', headers: { 'Authorization': `Bearer ${getToken()}` } }).then(r => r.json());

// SIP Trunks
export interface TrunkView {
  id: string;
  name: string;
  provider: string | null;
  host: string;
  port: number;
  transport: string;
  username: string | null;
  max_channels: number;
  active_channels: number;
  registration_required: boolean;
  status: string;
  did_count: number;
  created_at: string;
}

export interface CreateTrunkRequest {
  name: string;
  provider?: string;
  host: string;
  port?: number;
  transport?: string;
  username?: string;
  password?: string;
  max_channels?: number;
  registration_required?: boolean;
}

export interface UpdateTrunkRequest {
  name?: string;
  provider?: string;
  host?: string;
  port?: number;
  transport?: string;
  username?: string;
  password?: string;
  max_channels?: number;
  registration_required?: boolean;
  status?: string;
}

export const fetchTrunks = () => request<TrunkView[]>('/trunks');

export const createTrunk = (data: CreateTrunkRequest) =>
  fetch(`${BASE_URL}/trunks`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const updateTrunk = (id: string, data: UpdateTrunkRequest) =>
  fetch(`${BASE_URL}/trunks/${id}`, { method: 'PUT', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const deleteTrunk = (id: string) =>
  fetch(`${BASE_URL}/trunks/${id}`, { method: 'DELETE', headers: { 'Authorization': `Bearer ${getToken()}` } }).then(r => r.json());

// DID Numbers
export interface DidNumberView {
  id: string;
  number: string;
  trunk_id: string | null;
  trunk_name: string | null;
  assigned_to: string | null;
  assigned_type: string | null;
  description: string | null;
  created_at: string;
}

export interface CreateDidRequest {
  number: string;
  trunk_id?: string;
  assigned_to?: string;
  assigned_type?: string;
  description?: string;
}

export interface UpdateDidRequest {
  trunk_id?: string;
  assigned_to?: string;
  assigned_type?: string;
  description?: string;
}

export const fetchDids = () => request<DidNumberView[]>('/trunks/did');

export const createDid = (data: CreateDidRequest) =>
  fetch(`${BASE_URL}/trunks/did`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const updateDid = (id: string, data: UpdateDidRequest) =>
  fetch(`${BASE_URL}/trunks/did/${id}`, { method: 'PUT', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const deleteDid = (id: string) =>
  fetch(`${BASE_URL}/trunks/did/${id}`, { method: 'DELETE', headers: { 'Authorization': `Bearer ${getToken()}` } }).then(r => r.json());

// Schedules / Shifts
export interface ShiftView {
  id: string;
  name: string;
  start_time: string;
  end_time: string;
  break_minutes: number;
  color: string;
  created_at: string;
}

export interface ScheduleEntryView {
  id: string;
  agent_id: string;
  shift_id: string | null;
  date: string;
  status: string;
  check_in_at: string | null;
  check_out_at: string | null;
  notes: string | null;
}

export interface AttendanceSummary {
  scheduled: number;
  checked_in: number;
  checked_out: number;
  absent: number;
  leave: number;
}

export const fetchShifts = () => request<ShiftView[]>('/schedules/shifts');

export const createShift = (data: { name: string; start_time: string; end_time: string; break_minutes?: number; color?: string }) =>
  fetch(`${BASE_URL}/schedules/shifts`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const updateShift = (id: string, data: { name?: string; start_time?: string; end_time?: string; break_minutes?: number; color?: string }) =>
  fetch(`${BASE_URL}/schedules/shifts/${id}`, { method: 'PUT', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const deleteShift = (id: string) =>
  fetch(`${BASE_URL}/schedules/shifts/${id}`, { method: 'DELETE', headers: { 'Authorization': `Bearer ${getToken()}` } }).then(r => r.json());

export const fetchScheduleEntries = (params?: { date?: string; agent_id?: string }) => {
  const sp = new URLSearchParams();
  if (params?.date) sp.set('date', params.date);
  if (params?.agent_id) sp.set('agent_id', params.agent_id);
  const qs = sp.toString();
  return request<ScheduleEntryView[]>(`/schedules/entries${qs ? `?${qs}` : ''}`);
};

export const createScheduleEntry = (data: { agent_id: string; shift_id: string; date: string }) =>
  fetch(`${BASE_URL}/schedules/entries`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const updateScheduleEntry = (id: string, data: { status?: string; check_in_at?: string; check_out_at?: string; notes?: string }) =>
  fetch(`${BASE_URL}/schedules/entries/${id}`, { method: 'PUT', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const deleteScheduleEntry = (id: string) =>
  fetch(`${BASE_URL}/schedules/entries/${id}`, { method: 'DELETE', headers: { 'Authorization': `Bearer ${getToken()}` } }).then(r => r.json());

export const checkinEntry = (id: string) =>
  fetch(`${BASE_URL}/schedules/entries/${id}/checkin`, { method: 'POST', headers: { 'Authorization': `Bearer ${getToken()}` } }).then(r => r.json());

export const checkoutEntry = (id: string) =>
  fetch(`${BASE_URL}/schedules/entries/${id}/checkout`, { method: 'POST', headers: { 'Authorization': `Bearer ${getToken()}` } }).then(r => r.json());

export const fetchTodayAttendance = () => request<AttendanceSummary>('/schedules/today');

// Quality Check (QC)
export interface QcTemplateItemView {
  id: string;
  template_id: string;
  category: string;
  item_name: string;
  max_score: number;
  description: string | null;
  position: number;
}

export interface QcTemplateView {
  id: string;
  name: string;
  description: string | null;
  total_score: number;
  created_at: string;
  items: QcTemplateItemView[];
}

export interface QcScoreItemView {
  id: string;
  score_id: string;
  item_id: string | null;
  score: number;
  comment: string | null;
}

export interface QcScoreView {
  id: string;
  call_id: string;
  agent_id: string;
  template_id: string | null;
  scorer_id: string;
  total_score: number | null;
  max_score: number | null;
  comments: string | null;
  scored_at: string;
  items: QcScoreItemView[];
}

export interface CreateQcTemplateRequest {
  name: string;
  description?: string;
  total_score?: number;
}

export interface UpdateQcTemplateRequest {
  name?: string;
  description?: string;
  total_score?: number;
}

export interface CreateQcTemplateItemRequest {
  category: string;
  item_name: string;
  max_score: number;
  description?: string;
  position?: number;
}

export interface UpdateQcTemplateItemRequest {
  category?: string;
  item_name?: string;
  max_score?: number;
  description?: string;
  position?: number;
}

export interface SubmitQcScoreRequest {
  call_id: string;
  agent_id: string;
  template_id?: string;
  items: { item_id?: string; score: number; comment?: string }[];
  comments?: string;
}

export const fetchQcTemplates = () => request<QcTemplateView[]>('/quality/templates');

export const createQcTemplate = (data: CreateQcTemplateRequest) =>
  fetch(`${BASE_URL}/quality/templates`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const updateQcTemplate = (id: string, data: UpdateQcTemplateRequest) =>
  fetch(`${BASE_URL}/quality/templates/${id}`, { method: 'PUT', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const deleteQcTemplate = (id: string) =>
  fetch(`${BASE_URL}/quality/templates/${id}`, { method: 'DELETE', headers: { 'Authorization': `Bearer ${getToken()}` } }).then(r => r.json());

export const createQcTemplateItem = (templateId: string, data: CreateQcTemplateItemRequest) =>
  fetch(`${BASE_URL}/quality/templates/${templateId}/items`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const updateQcTemplateItem = (templateId: string, itemId: string, data: UpdateQcTemplateItemRequest) =>
  fetch(`${BASE_URL}/quality/templates/${templateId}/items/${itemId}`, { method: 'PUT', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const deleteQcTemplateItem = (templateId: string, itemId: string) =>
  fetch(`${BASE_URL}/quality/templates/${templateId}/items/${itemId}`, { method: 'DELETE', headers: { 'Authorization': `Bearer ${getToken()}` } }).then(r => r.json());

export const fetchQcScores = (params?: { agent_id?: string; call_id?: string }) => {
  const sp = new URLSearchParams();
  if (params?.agent_id) sp.set('agent_id', params.agent_id);
  if (params?.call_id) sp.set('call_id', params.call_id);
  const qs = sp.toString();
  return request<QcScoreView[]>(`/quality/scores${qs ? `?${qs}` : ''}`);
};

export const submitQcScore = (data: SubmitQcScoreRequest) =>
  fetch(`${BASE_URL}/quality/scores`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

// Reports
export interface DailyReport { date: string; total_calls: number; answered_calls: number; abandoned_calls: number; avg_duration_seconds: number; avg_wait_seconds: number; sla_percentage: number; }
export interface AgentPerformanceReport { agent_id: string; agent_name: string; total_calls: number; avg_duration_seconds: number; total_duration_seconds: number; }
export interface QueuePerformanceReport { queue_name: string; total_calls: number; avg_wait_seconds: number; max_wait_seconds: number; abandoned: number; }
export interface SummaryReport { period: string; total_calls: number; total_agents: number; avg_calls_per_agent: number; avg_duration: number; busiest_hour: string; top_agents: AgentPerformanceReport[]; queue_stats: QueuePerformanceReport[]; }

export const fetchDailyReport = (date: string) => request<DailyReport>(`/reports/daily?date=${date}`);
export const fetchAgentPerformance = (start: string, end: string, agentId?: string) => {
  const qs = new URLSearchParams({ start, end });
  if (agentId) qs.set('agent_id', agentId);
  return request<AgentPerformanceReport[]>(`/reports/agent-performance?${qs}`);
};
export const fetchQueuePerformance = (start: string, end: string) => request<QueuePerformanceReport[]>(`/reports/queue-performance?start=${start}&end=${end}`);
export const fetchSummaryReport = (start: string, end: string) => request<SummaryReport>(`/reports/summary?start=${start}&end=${end}`);

// Knowledge Base
export interface ArticleView {
  id: string;
  title: string;
  category: string | null;
  content: string;
  tags: string | null;
  is_published: boolean;
  view_count: number;
  created_by: string | null;
  created_at: string;
  updated_at: string;
}

export interface ScriptView {
  id: string;
  name: string;
  scenario: string | null;
  content: string;
  category: string | null;
  is_active: boolean;
  created_at: string;
}

export interface CreateArticleRequest {
  title: string;
  category?: string;
  content: string;
  tags?: string;
  is_published?: boolean;
}

export interface UpdateArticleRequest {
  title?: string;
  category?: string;
  content?: string;
  tags?: string;
  is_published?: boolean;
}

export interface CreateScriptRequest {
  name: string;
  scenario?: string;
  content: string;
  category?: string;
  is_active?: boolean;
}

export interface UpdateScriptRequest {
  name?: string;
  scenario?: string;
  content?: string;
  category?: string;
  is_active?: boolean;
}

export const fetchArticles = (params?: { category?: string; search?: string }) => {
  const qs = new URLSearchParams();
  if (params?.category) qs.set('category', params.category);
  if (params?.search) qs.set('search', params.search);
  const suffix = qs.toString() ? `?${qs}` : '';
  return request<ArticleView[]>(`/knowledge/articles${suffix}`);
};

export const createArticle = (data: CreateArticleRequest) =>
  fetch(`${BASE_URL}/knowledge/articles`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const updateArticle = (id: string, data: UpdateArticleRequest) =>
  fetch(`${BASE_URL}/knowledge/articles/${id}`, { method: 'PUT', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const deleteArticle = (id: string) =>
  fetch(`${BASE_URL}/knowledge/articles/${id}`, { method: 'DELETE', headers: { 'Authorization': `Bearer ${getToken()}` } }).then(r => r.json());

export const viewArticle = (id: string) =>
  fetch(`${BASE_URL}/knowledge/articles/${id}/view`, { method: 'POST', headers: { 'Authorization': `Bearer ${getToken()}` } }).then(r => r.json());

export const fetchScripts = (params?: { category?: string }) => {
  const qs = new URLSearchParams();
  if (params?.category) qs.set('category', params.category);
  const suffix = qs.toString() ? `?${qs}` : '';
  return request<ScriptView[]>(`/knowledge/scripts${suffix}`);
};

export const createScript = (data: CreateScriptRequest) =>
  fetch(`${BASE_URL}/knowledge/scripts`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const updateScript = (id: string, data: UpdateScriptRequest) =>
  fetch(`${BASE_URL}/knowledge/scripts/${id}`, { method: 'PUT', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const deleteScript = (id: string) =>
  fetch(`${BASE_URL}/knowledge/scripts/${id}`, { method: 'DELETE', headers: { 'Authorization': `Bearer ${getToken()}` } }).then(r => r.json());

// Softphone
export interface SoftphoneRegisterRequest {
  extension: string;
  domain: string;
  user_agent: string;
}

export interface SoftphoneRegisterResponse {
  registered: boolean;
  uri: string;
  expires: number;
}

export const softphoneRegister = (data: SoftphoneRegisterRequest): Promise<ApiResponse<SoftphoneRegisterResponse>> =>
  fetch(`${BASE_URL}/softphone/register`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

export const softphoneUnregister = (data: { extension: string; domain: string }): Promise<ApiResponse<string>> =>
  fetch(`${BASE_URL}/softphone/unregister`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify(data) }).then(r => r.json());

// AI Copilot
export interface AiKnowledgeRef { id: string; title: string; relevance: number; }
export interface AiQualityItem { name: string; checked: boolean; }
export interface AnalyzeResponse { intent: string; sentiment: number; sentiment_label: string; suggestion: string; knowledge_refs: AiKnowledgeRef[]; quality_items: AiQualityItem[]; }
export interface SuggestResponse { suggestion: string; scripts: { name: string; content: string }[]; }
export interface SummaryResponse { summary: string; key_topics: string[]; overall_sentiment: string; quality_score: number; }
export interface AiConfigResponse { enabled: boolean; provider: string; features: string[]; }

export const analyzeText = (text: string, callId?: string, agentId?: string): Promise<ApiResponse<AnalyzeResponse>> =>
  fetch(`${BASE_URL}/ai/analyze`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify({ text, call_id: callId, agent_id: agentId }) }).then(r => r.json());

export const suggestScript = (customerText: string, context?: string): Promise<ApiResponse<SuggestResponse>> =>
  fetch(`${BASE_URL}/ai/suggest`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify({ customer_text: customerText, context }) }).then(r => r.json());

export const summarizeCall = (callId: string, turns: { speaker: string; text: string }[]): Promise<ApiResponse<SummaryResponse>> =>
  fetch(`${BASE_URL}/ai/summarize`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${getToken()}` }, body: JSON.stringify({ call_id: callId, turns }) }).then(r => r.json());

export const fetchAiConfig = () => request<AiConfigResponse>('/ai/config');
