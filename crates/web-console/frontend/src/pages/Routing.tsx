import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table';
import { Route, Plus, Pencil, Trash2, CheckCircle } from 'lucide-react';
import {
  fetchRoutingConfig,
  updateRoutingConfig,
  fetchOverflowPolicies,
  createOverflowPolicy,
  updateOverflowPolicy,
  deleteOverflowPolicy,
} from '@/lib/api';
import type { RoutingConfigView, OverflowPolicyView } from '@/lib/api';

const STRATEGIES = ['RoundRobin', 'LeastRecentlyUsed', 'SkillBased', 'Random', 'Priority'] as const;
const LOAD_BALANCE_STRATEGIES = ['EqualDistribution', 'WeightedDistribution', 'LeastBusy', 'MostExperienced'] as const;
const CONDITION_TYPES = ['QueueSize', 'WaitTime', 'ServiceLevel', 'SystemUtilization', 'CustomerTier', 'BusinessHours', 'AfterHours', 'NoAgentsAvailable', 'Emergency', 'Custom'] as const;
const ACTION_TYPES = ['RouteToQueue', 'RouteToExternal', 'EnableCallbacks', 'PlayAnnouncement', 'RedirectToSelfService', 'ForwardToVoicemail', 'RejectCall', 'Custom'] as const;

interface PolicyFormState {
  name: string;
  condition_type: string;
  condition_value: string;
  action_type: string;
  action_value: string;
  priority: number;
  enabled: boolean;
}

const defaultPolicyForm: PolicyFormState = {
  name: '',
  condition_type: 'QueueSize',
  condition_value: '',
  action_type: 'RouteToQueue',
  action_value: '',
  priority: 0,
  enabled: true,
};

const selectClasses = 'flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-xs transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring';

export function Routing() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();

  // Config state
  const [configSaved, setConfigSaved] = useState(false);
  const [localConfig, setLocalConfig] = useState<RoutingConfigView | null>(null);

  // Policy dialog states
  const [createOpen, setCreateOpen] = useState(false);
  const [editOpen, setEditOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [editPolicyId, setEditPolicyId] = useState('');
  const [deletePolicyId, setDeletePolicyId] = useState('');
  const [policyForm, setPolicyForm] = useState<PolicyFormState>({ ...defaultPolicyForm });

  // Queries
  const { data: configData } = useQuery({
    queryKey: ['routing-config'],
    queryFn: fetchRoutingConfig,
  });

  const { data: policies } = useQuery({
    queryKey: ['overflow-policies'],
    queryFn: fetchOverflowPolicies,
  });

  const config = localConfig ?? configData ?? null;
  const policyList: OverflowPolicyView[] = policies ?? [];

  // Mutations
  const configMutation = useMutation({
    mutationFn: (data: Partial<RoutingConfigView>) => updateRoutingConfig(data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['routing-config'] });
      setConfigSaved(true);
      setLocalConfig(null);
      setTimeout(() => setConfigSaved(false), 3000);
    },
  });

  const createPolicyMutation = useMutation({
    mutationFn: (data: Omit<OverflowPolicyView, 'id'>) => createOverflowPolicy(data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['overflow-policies'] });
      setCreateOpen(false);
      setPolicyForm({ ...defaultPolicyForm });
    },
  });

  const updatePolicyMutation = useMutation({
    mutationFn: (params: { id: string; data: Omit<OverflowPolicyView, 'id'> }) =>
      updateOverflowPolicy(params.id, params.data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['overflow-policies'] });
      setEditOpen(false);
    },
  });

  const deletePolicyMutation = useMutation({
    mutationFn: (id: string) => deleteOverflowPolicy(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['overflow-policies'] });
      setDeleteOpen(false);
      setDeletePolicyId('');
    },
  });

  function handleConfigSave(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    if (config) {
      configMutation.mutate(config);
    }
  }

  function updateConfig(partial: Partial<RoutingConfigView>) {
    const base = config ?? {
      default_strategy: 'RoundRobin',
      enable_load_balancing: false,
      load_balance_strategy: 'EqualDistribution',
      enable_geographic_routing: false,
      enable_time_based_routing: false,
    };
    setLocalConfig({ ...base, ...partial });
  }

  function handleCreatePolicy(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    createPolicyMutation.mutate(policyForm);
  }

  function handleEditPolicy(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    updatePolicyMutation.mutate({ id: editPolicyId, data: policyForm });
  }

  function openEditPolicy(policy: OverflowPolicyView) {
    setEditPolicyId(policy.id);
    setPolicyForm({
      name: policy.name,
      condition_type: policy.condition_type,
      condition_value: policy.condition_value,
      action_type: policy.action_type,
      action_value: policy.action_value,
      priority: policy.priority,
      enabled: policy.enabled,
    });
    setEditOpen(true);
  }

  function openDeletePolicy(id: string) {
    setDeletePolicyId(id);
    setDeleteOpen(true);
  }

  function confirmDelete() {
    if (deletePolicyId) {
      deletePolicyMutation.mutate(deletePolicyId);
    }
  }

  function handleToggleEnabled(policy: OverflowPolicyView) {
    updatePolicyMutation.mutate({
      id: policy.id,
      data: {
        name: policy.name,
        condition_type: policy.condition_type,
        condition_value: policy.condition_value,
        action_type: policy.action_type,
        action_value: policy.action_value,
        priority: policy.priority,
        enabled: !policy.enabled,
      },
    });
  }

  const policyFormFields = (
    <div className="space-y-4">
      <div className="space-y-2">
        <Label htmlFor="policy-name">{t('routing.policyName')}</Label>
        <Input
          id="policy-name"
          value={policyForm.name}
          onChange={(e) => setPolicyForm({ ...policyForm, name: e.target.value })}
          required
        />
      </div>
      <div className="space-y-2">
        <Label htmlFor="policy-condition-type">{t('routing.condition')}</Label>
        <select
          id="policy-condition-type"
          className={selectClasses}
          value={policyForm.condition_type}
          onChange={(e) => setPolicyForm({ ...policyForm, condition_type: e.target.value })}
        >
          {CONDITION_TYPES.map((ct) => (
            <option key={ct} value={ct}>{ct}</option>
          ))}
        </select>
      </div>
      <div className="space-y-2">
        <Label htmlFor="policy-condition-value">{t('routing.conditionValue')}</Label>
        <Input
          id="policy-condition-value"
          value={policyForm.condition_value}
          onChange={(e) => setPolicyForm({ ...policyForm, condition_value: e.target.value })}
        />
      </div>
      <div className="space-y-2">
        <Label htmlFor="policy-action-type">{t('routing.action')}</Label>
        <select
          id="policy-action-type"
          className={selectClasses}
          value={policyForm.action_type}
          onChange={(e) => setPolicyForm({ ...policyForm, action_type: e.target.value })}
        >
          {ACTION_TYPES.map((at) => (
            <option key={at} value={at}>{at}</option>
          ))}
        </select>
      </div>
      <div className="space-y-2">
        <Label htmlFor="policy-action-value">{t('routing.actionValue')}</Label>
        <Input
          id="policy-action-value"
          value={policyForm.action_value}
          onChange={(e) => setPolicyForm({ ...policyForm, action_value: e.target.value })}
        />
      </div>
      <div className="space-y-2">
        <Label htmlFor="policy-priority">{t('routing.priority')}</Label>
        <Input
          id="policy-priority"
          type="number"
          min={0}
          value={policyForm.priority}
          onChange={(e) => setPolicyForm({ ...policyForm, priority: parseInt(e.target.value, 10) || 0 })}
        />
      </div>
      <div className="flex items-center justify-between">
        <Label htmlFor="policy-enabled">{t('routing.enabled')}</Label>
        <Switch
          id="policy-enabled"
          checked={policyForm.enabled}
          onCheckedChange={(val) => setPolicyForm({ ...policyForm, enabled: val })}
        />
      </div>
    </div>
  );

  return (
    <div className="p-6 space-y-6">
      {/* Header */}
      <div>
        <h1 className="text-2xl font-bold tracking-tight">{t('routing.title')}</h1>
        <p className="text-sm text-muted-foreground">{t('routing.subtitle')}</p>
      </div>

      {/* Section 1: Routing Strategy Config */}
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Route className="size-5" />
            {t('routing.strategy')}
          </CardTitle>
        </CardHeader>
        <CardContent>
          <form onSubmit={handleConfigSave} className="space-y-6">
            {/* Default Strategy */}
            <div className="space-y-2">
              <Label htmlFor="default-strategy">{t('routing.strategy')}</Label>
              <select
                id="default-strategy"
                className={selectClasses}
                value={config?.default_strategy ?? 'RoundRobin'}
                onChange={(e) => updateConfig({ default_strategy: e.target.value })}
              >
                {STRATEGIES.map((s) => (
                  <option key={s} value={s}>{s}</option>
                ))}
              </select>
            </div>

            {/* Load Balancing Switch */}
            <div className="flex items-center justify-between">
              <Label htmlFor="enable-lb">{t('routing.loadBalancing')}</Label>
              <Switch
                id="enable-lb"
                checked={config?.enable_load_balancing ?? false}
                onCheckedChange={(val) => updateConfig({ enable_load_balancing: val })}
              />
            </div>

            {/* Load Balance Strategy */}
            <div className="space-y-2">
              <Label htmlFor="lb-strategy">{t('routing.loadBalanceStrategy')}</Label>
              <select
                id="lb-strategy"
                className={selectClasses}
                value={config?.load_balance_strategy ?? 'EqualDistribution'}
                onChange={(e) => updateConfig({ load_balance_strategy: e.target.value })}
                disabled={!(config?.enable_load_balancing ?? false)}
              >
                {LOAD_BALANCE_STRATEGIES.map((s) => (
                  <option key={s} value={s}>{s}</option>
                ))}
              </select>
            </div>

            {/* Geographic Routing */}
            <div className="flex items-center justify-between">
              <Label htmlFor="enable-geo">{t('routing.geographicRouting')}</Label>
              <Switch
                id="enable-geo"
                checked={config?.enable_geographic_routing ?? false}
                onCheckedChange={(val) => updateConfig({ enable_geographic_routing: val })}
              />
            </div>

            {/* Time-Based Routing */}
            <div className="flex items-center justify-between">
              <Label htmlFor="enable-time">{t('routing.timeBasedRouting')}</Label>
              <Switch
                id="enable-time"
                checked={config?.enable_time_based_routing ?? false}
                onCheckedChange={(val) => updateConfig({ enable_time_based_routing: val })}
              />
            </div>

            {/* Save */}
            <div className="flex items-center gap-3">
              <Button type="submit" disabled={configMutation.isPending}>
                {t('routing.saveConfig')}
              </Button>
              {configSaved && (
                <span className="flex items-center gap-1.5 text-sm text-green-600">
                  <CheckCircle className="size-4" />
                  {t('routing.configSaved')}
                </span>
              )}
            </div>
          </form>
        </CardContent>
      </Card>

      {/* Section 2: Overflow Policies */}
      <Card>
        <CardHeader className="flex flex-row items-center justify-between">
          <CardTitle>{t('routing.overflow')}</CardTitle>
          <Dialog open={createOpen} onOpenChange={setCreateOpen}>
            <DialogTrigger render={<Button size="sm" />}>
              <Plus className="size-4 mr-1.5" />
              {t('routing.addPolicy')}
            </DialogTrigger>
            <DialogContent>
              <DialogHeader>
                <DialogTitle>{t('routing.addPolicy')}</DialogTitle>
                <DialogDescription>{t('routing.subtitle')}</DialogDescription>
              </DialogHeader>
              <form onSubmit={handleCreatePolicy}>
                {policyFormFields}
                <DialogFooter className="mt-4">
                  <Button type="button" variant="outline" onClick={() => setCreateOpen(false)}>
                    {t('common.cancel')}
                  </Button>
                  <Button type="submit" disabled={createPolicyMutation.isPending}>
                    {t('routing.addPolicy')}
                  </Button>
                </DialogFooter>
              </form>
            </DialogContent>
          </Dialog>
        </CardHeader>
        <CardContent>
          {policyList.length === 0 ? (
            <div className="py-12 text-center text-muted-foreground text-sm">
              {t('routing.noPolicies')}
            </div>
          ) : (
            <div className="border rounded-md overflow-hidden">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>{t('routing.policyName')}</TableHead>
                    <TableHead>{t('routing.condition')}</TableHead>
                    <TableHead>{t('routing.action')}</TableHead>
                    <TableHead>{t('routing.priority')}</TableHead>
                    <TableHead>{t('routing.enabled')}</TableHead>
                    <TableHead />
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {policyList.map((policy) => (
                    <TableRow key={policy.id}>
                      <TableCell className="font-medium">{policy.name}</TableCell>
                      <TableCell>
                        <div className="flex items-center gap-1.5">
                          <Badge variant="secondary" className="text-[10px]">{policy.condition_type}</Badge>
                          <span className="text-xs text-muted-foreground">{policy.condition_value}</span>
                        </div>
                      </TableCell>
                      <TableCell>
                        <div className="flex items-center gap-1.5">
                          <Badge variant="outline" className="text-[10px]">{policy.action_type}</Badge>
                          <span className="text-xs text-muted-foreground">{policy.action_value}</span>
                        </div>
                      </TableCell>
                      <TableCell className="font-mono text-sm">{policy.priority}</TableCell>
                      <TableCell>
                        <Switch
                          checked={policy.enabled}
                          onCheckedChange={() => handleToggleEnabled(policy)}
                        />
                      </TableCell>
                      <TableCell>
                        <div className="flex items-center gap-1">
                          <Button
                            variant="ghost"
                            size="sm"
                            className="h-7 w-7"
                            onClick={() => openEditPolicy(policy)}
                          >
                            <Pencil className="size-3.5" />
                          </Button>
                          <Button
                            variant="ghost"
                            size="sm"
                            className="h-7 w-7 text-destructive hover:text-destructive"
                            onClick={() => openDeletePolicy(policy.id)}
                          >
                            <Trash2 className="size-3.5" />
                          </Button>
                        </div>
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
          )}
        </CardContent>
      </Card>

      {/* Edit Policy Dialog */}
      <Dialog open={editOpen} onOpenChange={setEditOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('routing.editPolicy')}</DialogTitle>
            <DialogDescription>{t('routing.subtitle')}</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleEditPolicy}>
            {policyFormFields}
            <DialogFooter className="mt-4">
              <Button type="button" variant="outline" onClick={() => setEditOpen(false)}>
                {t('common.cancel')}
              </Button>
              <Button type="submit" disabled={updatePolicyMutation.isPending}>
                {t('common.save')}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* Delete Confirmation Dialog */}
      <Dialog open={deleteOpen} onOpenChange={setDeleteOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('routing.deletePolicy')}</DialogTitle>
            <DialogDescription>{t('routing.deleteConfirm')}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteOpen(false)}>
              {t('common.cancel')}
            </Button>
            <Button
              variant="destructive"
              onClick={confirmDelete}
              disabled={deletePolicyMutation.isPending}
            >
              {t('common.delete')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
