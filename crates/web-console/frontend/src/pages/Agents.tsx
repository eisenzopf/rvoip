import { useState, useMemo } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Checkbox } from '@/components/ui/checkbox';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog';
import { Users, UserCheck, UserX, Plus, Trash2, Pencil } from 'lucide-react';
import {
  fetchAgents,
  fetchDepartments,
  fetchSkills,
  createAgent,
  updateAgent,
  updateAgentStatus,
  deleteAgent,
  setAgentSkills,
  fetchAgentSkills,
} from '@/lib/api';
import type { AgentView, CreateAgentRequest, DepartmentView, SkillView, AgentSkillView } from '@/lib/api';

type FilterTab = 'all' | 'online' | 'busy' | 'offline';

const NO_DEPARTMENT = '__none__';

function statusColor(status: string): string {
  const s = status.toLowerCase();
  if (s === 'available' || s === 'online') return 'bg-green-500';
  if (s === 'busy' || s === 'on_call' || s === 'oncall') return 'bg-amber-500';
  return 'bg-muted-foreground';
}

function matchesTab(agent: AgentView, tab: FilterTab): boolean {
  if (tab === 'all') return true;
  const s = agent.status.toLowerCase();
  if (tab === 'online') return s === 'available' || s === 'online';
  if (tab === 'busy') return s === 'busy' || s === 'on_call' || s === 'oncall';
  return s === 'offline';
}

interface SkillEntry {
  skill_id: string;
  skill_name: string;
  proficiency: number;
  checked: boolean;
}

interface CreateFormState {
  display_name: string;
  extension: string;
  max_concurrent_calls: number;
  department: string;
}

interface EditFormState {
  display_name: string;
  extension: string;
  max_concurrent_calls: number;
  department: string;
}

const emptyCreateForm: CreateFormState = {
  display_name: '',
  extension: '',
  max_concurrent_calls: 1,
  department: '',
};

function buildSkillEntries(allSkills: SkillView[], agentSkills: AgentSkillView[]): SkillEntry[] {
  const map = new Map<string, AgentSkillView>();
  for (const as of agentSkills) {
    map.set(as.skill_id, as);
  }
  return allSkills.map((s) => {
    const existing = map.get(s.id);
    return {
      skill_id: s.id,
      skill_name: s.name,
      proficiency: existing?.proficiency ?? 3,
      checked: existing !== undefined,
    };
  });
}

function buildSkillEntriesFromNames(allSkills: SkillView[], skillNames: string[]): SkillEntry[] {
  const nameSet = new Set(skillNames.map((n) => n.toLowerCase()));
  return allSkills.map((s) => ({
    skill_id: s.id,
    skill_name: s.name,
    proficiency: 3,
    checked: nameSet.has(s.name.toLowerCase()),
  }));
}

export function Agents() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [tab, setTab] = useState<FilterTab>('all');

  // Create dialog
  const [createDialogOpen, setCreateDialogOpen] = useState(false);
  const [createForm, setCreateForm] = useState<CreateFormState>({ ...emptyCreateForm });
  const [createSkillEntries, setCreateSkillEntries] = useState<SkillEntry[]>([]);

  // Edit dialog
  const [editDialogOpen, setEditDialogOpen] = useState(false);
  const [editAgentId, setEditAgentId] = useState<string | null>(null);
  const [editForm, setEditForm] = useState<EditFormState>({ ...emptyCreateForm });
  const [editSkillEntries, setEditSkillEntries] = useState<SkillEntry[]>([]);

  // Delete dialog
  const [deleteDialogOpen, setDeleteDialogOpen] = useState(false);
  const [agentToDelete, setAgentToDelete] = useState<string | null>(null);

  const { data } = useQuery({
    queryKey: ['agents'],
    queryFn: fetchAgents,
    refetchInterval: 5000,
  });

  const { data: departments } = useQuery({
    queryKey: ['departments'],
    queryFn: fetchDepartments,
  });

  const { data: skills } = useQuery({
    queryKey: ['skills'],
    queryFn: fetchSkills,
  });

  const deptList: DepartmentView[] = departments ?? [];
  const skillList: SkillView[] = skills ?? [];

  const deptMap = useMemo(() => {
    const map = new Map<string, DepartmentView>();
    for (const d of deptList) {
      map.set(d.id, d);
      map.set(d.name, d);
    }
    return map;
  }, [deptList]);

  const createMutation = useMutation({
    mutationFn: async (params: { agent: CreateAgentRequest; skillEntries: SkillEntry[] }) => {
      const result = await createAgent(params.agent);
      const agentId = result?.data?.id ?? params.agent.id;
      if (agentId) {
        const selectedSkills = params.skillEntries
          .filter((e) => e.checked)
          .map((e) => ({ skill_id: e.skill_id, proficiency: e.proficiency }));
        if (selectedSkills.length > 0) {
          await setAgentSkills(agentId, selectedSkills);
        }
      }
      return result;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['agents'] });
      setCreateDialogOpen(false);
      setCreateForm({ ...emptyCreateForm });
      setCreateSkillEntries([]);
    },
  });

  const updateMutation = useMutation({
    mutationFn: async (vars: { id: string; data: Partial<CreateAgentRequest>; skillEntries: SkillEntry[] }) => {
      const result = await updateAgent(vars.id, vars.data);
      const selectedSkills = vars.skillEntries
        .filter((e) => e.checked)
        .map((e) => ({ skill_id: e.skill_id, proficiency: e.proficiency }));
      await setAgentSkills(vars.id, selectedSkills);
      return result;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['agents'] });
      setEditDialogOpen(false);
      setEditAgentId(null);
    },
  });

  const statusMutation = useMutation({
    mutationFn: (vars: { id: string; status: string }) =>
      updateAgentStatus(vars.id, vars.status),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['agents'] });
    },
  });

  const deleteMutation = useMutation({
    mutationFn: deleteAgent,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['agents'] });
      setDeleteDialogOpen(false);
      setAgentToDelete(null);
    },
  });

  const agents: AgentView[] = data?.agents ?? [];

  const counts = useMemo(() => {
    let online = 0;
    let busy = 0;
    for (const a of agents) {
      const s = a.status.toLowerCase();
      if (s === 'available' || s === 'online') online++;
      else if (s === 'busy' || s === 'on_call' || s === 'oncall') busy++;
    }
    return { total: agents.length, online, busy, offline: agents.length - online - busy };
  }, [agents]);

  const filtered = useMemo(
    () => agents.filter((a) => matchesTab(a, tab)),
    [agents, tab],
  );

  const tabs: { key: FilterTab; labelKey: string; count: number }[] = [
    { key: 'all', labelKey: 'agents.all', count: counts.total },
    { key: 'online', labelKey: 'agents.online', count: counts.online },
    { key: 'busy', labelKey: 'agents.busy', count: counts.busy },
    { key: 'offline', labelKey: 'agents.offline', count: counts.offline },
  ];

  function getDeptDisplayName(dept: string | undefined): string | undefined {
    if (!dept) return undefined;
    const found = deptMap.get(dept);
    return found?.name ?? dept;
  }

  function handleCreateOpen() {
    setCreateForm({ ...emptyCreateForm });
    setCreateSkillEntries(
      skillList.map((s) => ({
        skill_id: s.id,
        skill_name: s.name,
        proficiency: 3,
        checked: false,
      })),
    );
    setCreateDialogOpen(true);
  }

  function handleCreateSubmit(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const selectedSkillNames = createSkillEntries
      .filter((e) => e.checked)
      .map((e) => e.skill_name);
    const deptValue = createForm.department === NO_DEPARTMENT ? undefined : createForm.department;
    createMutation.mutate({
      agent: {
        display_name: createForm.display_name,
        skills: selectedSkillNames,
        max_concurrent_calls: createForm.max_concurrent_calls,
        department: deptValue || undefined,
        extension: createForm.extension || undefined,
      },
      skillEntries: createSkillEntries,
    });
  }

  async function handleEditClick(agent: AgentView) {
    setEditAgentId(agent.id);
    const deptId = agent.department || '';
    setEditForm({
      display_name: agent.display_name || agent.id,
      extension: agent.extension || '',
      max_concurrent_calls: agent.max_calls,
      department: deptId || NO_DEPARTMENT,
    });

    // Try to fetch agent's skill proficiencies, fallback to name-based matching
    try {
      const agentSkills = await fetchAgentSkills(agent.id);
      setEditSkillEntries(buildSkillEntries(skillList, agentSkills));
    } catch {
      setEditSkillEntries(buildSkillEntriesFromNames(skillList, agent.skills));
    }
    setEditDialogOpen(true);
  }

  function handleEditSubmit(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    if (!editAgentId) return;
    const selectedSkillNames = editSkillEntries
      .filter((e) => e.checked)
      .map((e) => e.skill_name);
    const deptValue = editForm.department === NO_DEPARTMENT ? undefined : editForm.department;
    updateMutation.mutate({
      id: editAgentId,
      data: {
        display_name: editForm.display_name,
        skills: selectedSkillNames,
        max_concurrent_calls: editForm.max_concurrent_calls,
        department: deptValue || undefined,
      },
      skillEntries: editSkillEntries,
    });
  }

  function handleStatusChange(agentId: string, newStatus: string) {
    statusMutation.mutate({ id: agentId, status: newStatus });
  }

  function handleDeleteClick(agentId: string) {
    setAgentToDelete(agentId);
    setDeleteDialogOpen(true);
  }

  function confirmDelete() {
    if (agentToDelete) {
      deleteMutation.mutate(agentToDelete);
    }
  }

  function updateCreateSkill(index: number, updates: Partial<SkillEntry>) {
    setCreateSkillEntries((prev) => {
      const next = [...prev];
      next[index] = { ...next[index], ...updates };
      return next;
    });
  }

  function updateEditSkill(index: number, updates: Partial<SkillEntry>) {
    setEditSkillEntries((prev) => {
      const next = [...prev];
      next[index] = { ...next[index], ...updates };
      return next;
    });
  }

  const statusOptions = ['Available', 'Busy', 'Offline'];

  function renderSkillsSelector(
    entries: SkillEntry[],
    onUpdate: (index: number, updates: Partial<SkillEntry>) => void,
  ) {
    if (entries.length === 0) {
      return (
        <p className="text-xs text-muted-foreground">{t('skills.noSkills')}</p>
      );
    }
    return (
      <div className="space-y-2 max-h-48 overflow-y-auto rounded-md border p-2">
        {entries.map((entry, idx) => (
          <div key={entry.skill_id} className="flex items-center gap-2">
            <Checkbox
              checked={entry.checked}
              onCheckedChange={(checked) => onUpdate(idx, { checked })}
            />
            <span className="text-sm flex-1 min-w-0 truncate">{entry.skill_name}</span>
            {entry.checked && (
              <Select
                value={entry.proficiency}
                onValueChange={(val) => onUpdate(idx, { proficiency: val as number })}
              >
                <SelectTrigger size="sm" className="w-16 h-7 text-xs">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {[1, 2, 3, 4, 5].map((n) => (
                    <SelectItem key={n} value={n}>
                      {n}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            )}
          </div>
        ))}
      </div>
    );
  }

  return (
    <div className="p-6 space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">{t('agents.title')}</h1>
          <p className="text-sm text-muted-foreground">
            {t('agents.subtitle')}
          </p>
        </div>

        {/* Add Agent Dialog */}
        <Dialog open={createDialogOpen} onOpenChange={setCreateDialogOpen}>
          <DialogTrigger render={<Button size="sm" onClick={handleCreateOpen} />}>
            <Plus className="size-4 mr-1.5" />
            {t('agents.addAgent')}
          </DialogTrigger>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>{t('agents.addAgent')}</DialogTitle>
              <DialogDescription>{t('agents.subtitle')}</DialogDescription>
            </DialogHeader>
            <form onSubmit={handleCreateSubmit} className="space-y-4">
              <div className="space-y-2">
                <Label htmlFor="create-display-name">{t('agents.form.displayName')}</Label>
                <Input
                  id="create-display-name"
                  value={createForm.display_name}
                  onChange={(e) => setCreateForm({ ...createForm, display_name: e.target.value })}
                  required
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="create-extension">{t('agents.extension')}</Label>
                <Input
                  id="create-extension"
                  value={createForm.extension}
                  onChange={(e) => setCreateForm({ ...createForm, extension: e.target.value })}
                  placeholder={t('agents.extensionPlaceholder')}
                />
              </div>
              <div className="space-y-2">
                <Label>{t('agents.department')}</Label>
                <Select
                  value={createForm.department || NO_DEPARTMENT}
                  onValueChange={(val) =>
                    setCreateForm({ ...createForm, department: val as string })
                  }
                >
                  <SelectTrigger className="w-full">
                    <SelectValue placeholder={t('agents.selectDepartment')} />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value={NO_DEPARTMENT}>{t('agents.noDepartment')}</SelectItem>
                    {deptList.map((dept) => (
                      <SelectItem key={dept.id} value={dept.id}>
                        {dept.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="space-y-2">
                <Label>{t('agents.selectSkills')}</Label>
                {renderSkillsSelector(createSkillEntries, updateCreateSkill)}
              </div>
              <div className="space-y-2">
                <Label htmlFor="create-max-calls">{t('agents.form.maxCalls')}</Label>
                <Input
                  id="create-max-calls"
                  type="number"
                  min={1}
                  max={10}
                  value={createForm.max_concurrent_calls}
                  onChange={(e) => setCreateForm({ ...createForm, max_concurrent_calls: parseInt(e.target.value, 10) || 1 })}
                  required
                />
              </div>
              <p className="text-xs text-muted-foreground">{t('agents.autoGenerated')}: ID, SIP URI</p>
              <DialogFooter>
                <Button type="button" variant="outline" onClick={() => setCreateDialogOpen(false)}>
                  {t('agents.form.cancel')}
                </Button>
                <Button type="submit" disabled={createMutation.isPending}>
                  {t('agents.form.create')}
                </Button>
              </DialogFooter>
            </form>
          </DialogContent>
        </Dialog>
      </div>

      {/* Stats */}
      <div className="grid gap-4 md:grid-cols-3">
        <Card>
          <CardHeader className="flex flex-row items-center justify-between pb-2">
            <CardTitle className="text-sm font-medium text-muted-foreground">{t('agents.total')}</CardTitle>
            <Users className="size-4 text-muted-foreground" />
          </CardHeader>
          <CardContent>
            <div className="text-2xl font-bold font-mono">{counts.total}</div>
          </CardContent>
        </Card>
        <Card>
          <CardHeader className="flex flex-row items-center justify-between pb-2">
            <CardTitle className="text-sm font-medium text-muted-foreground">{t('agents.online')}</CardTitle>
            <UserCheck className="size-4 text-green-500" />
          </CardHeader>
          <CardContent>
            <div className="text-2xl font-bold font-mono text-green-500">{counts.online}</div>
          </CardContent>
        </Card>
        <Card>
          <CardHeader className="flex flex-row items-center justify-between pb-2">
            <CardTitle className="text-sm font-medium text-muted-foreground">{t('agents.busy')}</CardTitle>
            <UserX className="size-4 text-amber-500" />
          </CardHeader>
          <CardContent>
            <div className="text-2xl font-bold font-mono text-amber-500">{counts.busy}</div>
          </CardContent>
        </Card>
      </div>

      {/* Filter Tabs */}
      <div className="flex items-center gap-1 border-b">
        {tabs.map((item) => (
          <Button
            key={item.key}
            variant={tab === item.key ? 'default' : 'ghost'}
            size="sm"
            className="rounded-b-none"
            onClick={() => setTab(item.key)}
          >
            {t(item.labelKey)}
            <Badge variant="secondary" className="ml-1.5 text-[10px] px-1.5">
              {item.count}
            </Badge>
          </Button>
        ))}
      </div>

      {/* Agent Grid */}
      {filtered.length === 0 ? (
        <Card>
          <CardContent className="py-16 text-center text-muted-foreground text-sm">
            {t('agents.noAgents')}
          </CardContent>
        </Card>
      ) : (
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
          {filtered.map((agent) => (
            <Card key={agent.id}>
              <CardContent className="p-4 space-y-3">
                {/* Top row: avatar + info */}
                <div className="flex items-center gap-3">
                  <div className="relative flex h-10 w-10 items-center justify-center rounded-md bg-muted text-sm font-semibold shrink-0">
                    {(agent.display_name || agent.id).slice(0, 2).toUpperCase()}
                    <span
                      className={`absolute -bottom-0.5 -right-0.5 h-2.5 w-2.5 rounded-full border-2 border-background ${statusColor(agent.status)}`}
                    />
                  </div>
                  <div className="min-w-0 flex-1">
                    <p className="text-sm font-medium truncate">{agent.display_name || agent.id}</p>
                    <div className="flex items-center gap-2 text-xs text-muted-foreground">
                      <span className="font-mono">{agent.id}</span>
                      {agent.extension && (
                        <Badge variant="outline" className="text-[10px] px-1">
                          {t('agents.extension')}: {agent.extension}
                        </Badge>
                      )}
                    </div>
                  </div>
                  <div className="flex items-center gap-1 shrink-0">
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-7 w-7"
                      onClick={() => handleEditClick(agent)}
                    >
                      <Pencil className="size-3.5" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-7 w-7 text-destructive hover:text-destructive"
                      onClick={() => handleDeleteClick(agent.id)}
                    >
                      <Trash2 className="size-3.5" />
                    </Button>
                  </div>
                </div>

                {/* Department badge */}
                {agent.department && (
                  <Badge variant="secondary" className="text-[10px] bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200">
                    {getDeptDisplayName(agent.department)}
                  </Badge>
                )}

                {/* SIP URI */}
                <p className="text-xs text-muted-foreground font-mono truncate">
                  {agent.sip_uri}
                </p>

                {/* Status quick-change */}
                <div className="flex items-center gap-1">
                  {statusOptions.map((s) => {
                    const current = agent.status.toLowerCase() === s.toLowerCase();
                    return (
                      <Button
                        key={s}
                        variant={current ? 'default' : 'outline'}
                        size="sm"
                        className="h-6 text-[10px] px-2"
                        disabled={current || statusMutation.isPending}
                        onClick={() => handleStatusChange(agent.id, s)}
                      >
                        {t(`agents.${s.toLowerCase()}`)}
                      </Button>
                    );
                  })}
                </div>

                {/* Skills with proficiency stars */}
                {agent.skills.length > 0 && (
                  <div className="flex flex-wrap gap-1">
                    {agent.skills.map((skill) => (
                      <Badge key={skill} variant="secondary" className="text-[10px] gap-0.5">
                        {skill}
                      </Badge>
                    ))}
                  </div>
                )}

                {/* Calls + Performance */}
                <div className="space-y-2">
                  <div className="flex justify-between text-xs">
                    <span className="text-muted-foreground">
                      {t('agents.xCalls', { current: agent.current_calls, max: agent.max_calls })}
                    </span>
                    <span className="font-mono">
                      {Math.round(agent.performance_score * 100)}%
                    </span>
                  </div>
                  <div className="h-1.5 rounded-full bg-muted overflow-hidden">
                    <div
                      className={`h-full rounded-full transition-all ${
                        agent.performance_score >= 0.8
                          ? 'bg-green-500'
                          : agent.performance_score >= 0.5
                            ? 'bg-amber-500'
                            : 'bg-red-500'
                      }`}
                      style={{ width: `${Math.round(agent.performance_score * 100)}%` }}
                    />
                  </div>
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      )}

      {/* Edit Agent Dialog */}
      <Dialog open={editDialogOpen} onOpenChange={setEditDialogOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('agents.editAgent')}</DialogTitle>
            <DialogDescription>{editAgentId ?? ''}</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleEditSubmit} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="edit-display-name">{t('agents.form.displayName')}</Label>
              <Input
                id="edit-display-name"
                value={editForm.display_name}
                onChange={(e) => setEditForm({ ...editForm, display_name: e.target.value })}
                required
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="edit-extension">{t('agents.extension')}</Label>
              <Input
                id="edit-extension"
                value={editForm.extension}
                readOnly
                className="bg-muted"
              />
            </div>
            <div className="space-y-2">
              <Label>{t('agents.department')}</Label>
              <Select
                value={editForm.department || NO_DEPARTMENT}
                onValueChange={(val) =>
                  setEditForm({ ...editForm, department: val as string })
                }
              >
                <SelectTrigger className="w-full">
                  <SelectValue placeholder={t('agents.selectDepartment')} />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={NO_DEPARTMENT}>{t('agents.noDepartment')}</SelectItem>
                  {deptList.map((dept) => (
                    <SelectItem key={dept.id} value={dept.id}>
                      {dept.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-2">
              <Label>{t('agents.selectSkills')}</Label>
              {renderSkillsSelector(editSkillEntries, updateEditSkill)}
            </div>
            <div className="space-y-2">
              <Label htmlFor="edit-max-calls">{t('agents.form.maxCalls')}</Label>
              <Input
                id="edit-max-calls"
                type="number"
                min={1}
                max={10}
                value={editForm.max_concurrent_calls}
                onChange={(e) => setEditForm({ ...editForm, max_concurrent_calls: parseInt(e.target.value, 10) || 1 })}
                required
              />
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setEditDialogOpen(false)}>
                {t('agents.form.cancel')}
              </Button>
              <Button type="submit" disabled={updateMutation.isPending}>
                {t('agents.form.save')}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* Delete Confirmation Dialog */}
      <Dialog open={deleteDialogOpen} onOpenChange={setDeleteDialogOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('agents.deleteAgent')}</DialogTitle>
            <DialogDescription>
              {t('agents.deleteConfirm')}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteDialogOpen(false)}>
              {t('common.cancel')}
            </Button>
            <Button
              variant="destructive"
              onClick={confirmDelete}
              disabled={deleteMutation.isPending}
            >
              {t('common.delete')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
