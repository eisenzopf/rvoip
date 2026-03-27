import { useState, useMemo } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';
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
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table';
import {
  ListOrdered,
  Clock,
  AlertTriangle,
  Plus,
  Pencil,
  Trash2,
  ChevronDown,
  ChevronUp,
  UserPlus,
} from 'lucide-react';
import {
  fetchQueues,
  fetchAgents,
  createQueue,
  updateQueue,
  deleteQueue,
  fetchQueueCalls,
  assignQueueCall,
} from '@/lib/api';
import type { QueueView, QueueConfigView, QueuedCallView, AgentView } from '@/lib/api';
import { useAuth } from '@/hooks/useAuth';

function loadLevel(totalCalls: number): { label: string; color: string; barColor: string } {
  if (totalCalls <= 3) return { label: 'Low', color: 'text-green-600', barColor: 'bg-green-500' };
  if (totalCalls <= 8) return { label: 'Medium', color: 'text-amber-600', barColor: 'bg-amber-500' };
  return { label: 'High', color: 'text-red-600', barColor: 'bg-red-500' };
}

function formatWait(secs: number): string {
  if (secs < 60) return `${secs}s`;
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return `${m}m ${s}s`;
}

interface EditFormState {
  default_max_wait_time: number;
  max_queue_size: number;
  enable_priorities: boolean;
  enable_overflow: boolean;
  announcement_interval: number;
}

const defaultEditForm: EditFormState = {
  default_max_wait_time: 300,
  max_queue_size: 100,
  enable_priorities: false,
  enable_overflow: false,
  announcement_interval: 30,
};

export function Queues() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const { hasRole, hasAnyRole } = useAuth();
  const isAdmin = hasRole('admin');
  const canAssign = hasAnyRole(['admin', 'supervisor']);

  // Dialog states
  const [createOpen, setCreateOpen] = useState(false);
  const [editOpen, setEditOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [assignOpen, setAssignOpen] = useState(false);

  // Form states
  const [newQueueId, setNewQueueId] = useState('');
  const [editQueueId, setEditQueueId] = useState('');
  const [editForm, setEditForm] = useState<EditFormState>({ ...defaultEditForm });
  const [deleteTargetId, setDeleteTargetId] = useState('');
  const [expandedQueue, setExpandedQueue] = useState<string | null>(null);
  const [assignCallId, setAssignCallId] = useState('');
  const [assignQueueId, setAssignQueueId] = useState('');
  const [selectedAgentId, setSelectedAgentId] = useState('');

  // Queries
  const { data } = useQuery({
    queryKey: ['queues'],
    queryFn: fetchQueues,
    refetchInterval: 5000,
  });

  const { data: agentsData } = useQuery({
    queryKey: ['agents'],
    queryFn: fetchAgents,
    enabled: canAssign,
  });

  const { data: queueCalls } = useQuery({
    queryKey: ['queue-calls', expandedQueue],
    queryFn: () => fetchQueueCalls(expandedQueue ?? ''),
    enabled: expandedQueue !== null,
    refetchInterval: 5000,
  });

  const queues: QueueView[] = data?.queues ?? [];
  const configs: QueueConfigView[] = data?.configs ?? [];
  const totalWaiting = data?.total_waiting ?? 0;
  const agents: AgentView[] = agentsData?.agents ?? [];

  const configMap = useMemo(() => {
    const map = new Map<string, QueueConfigView>();
    for (const c of configs) {
      map.set(c.queue_id, c);
    }
    return map;
  }, [configs]);

  // Mutations
  const createMutation = useMutation({
    mutationFn: (queueId: string) => createQueue(queueId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['queues'] });
      setCreateOpen(false);
      setNewQueueId('');
    },
  });

  const updateMutation = useMutation({
    mutationFn: (params: { id: string; data: Partial<QueueConfigView> }) =>
      updateQueue(params.id, params.data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['queues'] });
      setEditOpen(false);
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => deleteQueue(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['queues'] });
      setDeleteOpen(false);
      setDeleteTargetId('');
    },
  });

  const assignMutation = useMutation({
    mutationFn: (params: { queueId: string; callId: string; agentId: string }) =>
      assignQueueCall(params.queueId, params.callId, params.agentId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['queue-calls', expandedQueue] });
      queryClient.invalidateQueries({ queryKey: ['queues'] });
      setAssignOpen(false);
      setSelectedAgentId('');
    },
  });

  function handleCreate(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    createMutation.mutate(newQueueId);
  }

  function handleEdit(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    updateMutation.mutate({ id: editQueueId, data: editForm });
  }

  function openEdit(queueId: string) {
    const cfg = configMap.get(queueId);
    setEditQueueId(queueId);
    setEditForm(cfg ? {
      default_max_wait_time: cfg.default_max_wait_time,
      max_queue_size: cfg.max_queue_size,
      enable_priorities: cfg.enable_priorities,
      enable_overflow: cfg.enable_overflow,
      announcement_interval: cfg.announcement_interval,
    } : { ...defaultEditForm });
    setEditOpen(true);
  }

  function openDelete(queueId: string) {
    setDeleteTargetId(queueId);
    setDeleteOpen(true);
  }

  function confirmDelete() {
    if (deleteTargetId) {
      deleteMutation.mutate(deleteTargetId);
    }
  }

  function toggleExpand(queueId: string) {
    setExpandedQueue(prev => prev === queueId ? null : queueId);
  }

  function openAssign(queueId: string, callId: string) {
    setAssignQueueId(queueId);
    setAssignCallId(callId);
    setSelectedAgentId('');
    setAssignOpen(true);
  }

  function confirmAssign() {
    if (selectedAgentId && assignQueueId && assignCallId) {
      assignMutation.mutate({ queueId: assignQueueId, callId: assignCallId, agentId: selectedAgentId });
    }
  }

  const calls: QueuedCallView[] = queueCalls ?? [];

  return (
    <div className="p-6 space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">{t('queues.title')}</h1>
          <p className="text-sm text-muted-foreground">
            {t('queues.subtitle')}
          </p>
        </div>

        {isAdmin && (
          <Dialog open={createOpen} onOpenChange={setCreateOpen}>
            <DialogTrigger render={<Button size="sm" />}>
              <Plus className="size-4 mr-1.5" />
              {t('queues.createQueue')}
            </DialogTrigger>
            <DialogContent>
              <DialogHeader>
                <DialogTitle>{t('queues.createQueue')}</DialogTitle>
                <DialogDescription>{t('queues.subtitle')}</DialogDescription>
              </DialogHeader>
              <form onSubmit={handleCreate} className="space-y-4">
                <div className="space-y-2">
                  <Label htmlFor="new-queue-id">{t('queues.queueId')}</Label>
                  <Input
                    id="new-queue-id"
                    value={newQueueId}
                    onChange={(e) => setNewQueueId(e.target.value)}
                    required
                    placeholder="support-queue"
                  />
                </div>
                <DialogFooter>
                  <Button type="button" variant="outline" onClick={() => setCreateOpen(false)}>
                    {t('common.cancel')}
                  </Button>
                  <Button type="submit" disabled={createMutation.isPending}>
                    {t('queues.createQueue')}
                  </Button>
                </DialogFooter>
              </form>
            </DialogContent>
          </Dialog>
        )}
      </div>

      {/* Summary Stats */}
      <div className="grid gap-4 md:grid-cols-3">
        <Card>
          <CardHeader className="flex flex-row items-center justify-between pb-2">
            <CardTitle className="text-sm font-medium text-muted-foreground">
              {t('queues.totalWaiting')}
            </CardTitle>
            <ListOrdered className="size-4 text-amber-500" />
          </CardHeader>
          <CardContent>
            <div className="text-2xl font-bold font-mono text-amber-500">
              {totalWaiting}
            </div>
          </CardContent>
        </Card>
        <Card>
          <CardHeader className="flex flex-row items-center justify-between pb-2">
            <CardTitle className="text-sm font-medium text-muted-foreground">
              {t('queues.activeQueues')}
            </CardTitle>
            <ListOrdered className="size-4 text-muted-foreground" />
          </CardHeader>
          <CardContent>
            <div className="text-2xl font-bold font-mono">{queues.length}</div>
          </CardContent>
        </Card>
        <Card>
          <CardHeader className="flex flex-row items-center justify-between pb-2">
            <CardTitle className="text-sm font-medium text-muted-foreground">
              {t('queues.longestWait')}
            </CardTitle>
            <AlertTriangle className="size-4 text-red-500" />
          </CardHeader>
          <CardContent>
            <div className="text-2xl font-bold font-mono text-red-500">
              {queues.length > 0
                ? formatWait(Math.max(...queues.map((q) => q.longest_wait_secs)))
                : '\u2014'}
            </div>
          </CardContent>
        </Card>
      </div>

      {/* Queue Cards */}
      {queues.length === 0 ? (
        <Card>
          <CardContent className="py-16 text-center text-muted-foreground text-sm">
            {t('queues.noQueues')}
          </CardContent>
        </Card>
      ) : (
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
          {queues.map((q) => {
            const level = loadLevel(q.total_calls);
            const barPercent = Math.min(q.total_calls * 10, 100);
            const cfg = configMap.get(q.queue_id);
            const isExpanded = expandedQueue === q.queue_id;

            return (
              <Card key={q.queue_id} className="col-span-1">
                <CardHeader className="flex flex-row items-center justify-between pb-2">
                  <CardTitle className="text-sm font-semibold truncate">
                    {q.queue_id}
                  </CardTitle>
                  <div className="flex items-center gap-1">
                    <Badge
                      variant="outline"
                      className={`text-[10px] ${level.color}`}
                    >
                      {level.label}
                    </Badge>
                    {isAdmin && (
                      <>
                        <Button
                          variant="ghost"
                          size="sm"
                          className="h-7 w-7"
                          onClick={() => openEdit(q.queue_id)}
                        >
                          <Pencil className="size-3.5" />
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          className="h-7 w-7 text-destructive hover:text-destructive"
                          onClick={() => openDelete(q.queue_id)}
                        >
                          <Trash2 className="size-3.5" />
                        </Button>
                      </>
                    )}
                  </div>
                </CardHeader>
                <CardContent className="space-y-4">
                  {/* Load bar */}
                  <div className="space-y-1.5">
                    <div className="flex justify-between text-xs text-muted-foreground">
                      <span>Load</span>
                      <span className="font-mono">{q.total_calls} calls</span>
                    </div>
                    <div className="h-2 rounded-full bg-muted overflow-hidden">
                      <div
                        className={`h-full rounded-full transition-all ${level.barColor}`}
                        style={{ width: `${barPercent}%` }}
                      />
                    </div>
                  </div>

                  {/* Stats */}
                  <div className="grid grid-cols-2 gap-3">
                    <div className="space-y-0.5">
                      <p className="text-[11px] text-muted-foreground">{t('queues.waiting')}</p>
                      <p className="text-lg font-bold font-mono">{q.total_calls}</p>
                    </div>
                    <div className="space-y-0.5">
                      <p className="text-[11px] text-muted-foreground">{t('queues.avgWait')}</p>
                      <div className="flex items-center gap-1">
                        <Clock className="size-3 text-muted-foreground" />
                        <p className="text-lg font-bold font-mono">
                          {formatWait(q.avg_wait_secs)}
                        </p>
                      </div>
                    </div>
                  </div>

                  {/* Longest wait */}
                  <div className="flex items-center justify-between rounded-md bg-muted/50 px-3 py-2">
                    <span className="text-xs text-muted-foreground">{t('queues.longestWaitTime')}</span>
                    <span className={`text-xs font-mono font-medium ${
                      q.longest_wait_secs > 120 ? 'text-red-500' : 'text-muted-foreground'
                    }`}>
                      {formatWait(q.longest_wait_secs)}
                    </span>
                  </div>

                  {/* Config badges */}
                  {cfg && (
                    <div className="flex flex-wrap gap-1.5">
                      <Badge variant="secondary" className="text-[10px]">
                        {t('queues.maxWait')}: {cfg.default_max_wait_time}
                      </Badge>
                      <Badge variant="secondary" className="text-[10px]">
                        {t('queues.maxSize')}: {cfg.max_queue_size}
                      </Badge>
                      {cfg.enable_priorities && (
                        <Badge variant="secondary" className="text-[10px]">
                          {t('queues.priorities')}
                        </Badge>
                      )}
                      {cfg.enable_overflow && (
                        <Badge variant="secondary" className="text-[10px]">
                          {t('queues.overflow')}
                        </Badge>
                      )}
                    </div>
                  )}

                  {/* View Calls button */}
                  <Button
                    variant="outline"
                    size="sm"
                    className="w-full"
                    onClick={() => toggleExpand(q.queue_id)}
                  >
                    {isExpanded ? (
                      <ChevronUp className="size-4 mr-1.5" />
                    ) : (
                      <ChevronDown className="size-4 mr-1.5" />
                    )}
                    {t('queues.queuedCalls')}
                  </Button>

                  {/* Expanded calls list */}
                  {isExpanded && (
                    <div className="border rounded-md overflow-hidden">
                      {calls.length === 0 ? (
                        <div className="py-6 text-center text-muted-foreground text-xs">
                          {t('queues.noQueuedCalls')}
                        </div>
                      ) : (
                        <Table>
                          <TableHeader>
                            <TableRow>
                              <TableHead className="text-xs">ID</TableHead>
                              <TableHead className="text-xs">{t('calls.from')}</TableHead>
                              <TableHead className="text-xs">{t('calls.status')}</TableHead>
                              <TableHead className="text-xs">{t('calls.priority')}</TableHead>
                              {canAssign && <TableHead className="text-xs" />}
                            </TableRow>
                          </TableHeader>
                          <TableBody>
                            {calls.map((call) => (
                              <TableRow key={call.session_id}>
                                <TableCell className="text-xs font-mono truncate max-w-[80px]">
                                  {call.session_id.slice(0, 8)}
                                </TableCell>
                                <TableCell className="text-xs truncate max-w-[100px]">
                                  {call.from}
                                </TableCell>
                                <TableCell>
                                  <Badge variant="outline" className="text-[10px]">
                                    {call.status}
                                  </Badge>
                                </TableCell>
                                <TableCell className="text-xs font-mono">
                                  {call.priority}
                                </TableCell>
                                {canAssign && (
                                  <TableCell>
                                    <Button
                                      variant="ghost"
                                      size="sm"
                                      className="h-6"
                                      onClick={() => openAssign(q.queue_id, call.session_id)}
                                    >
                                      <UserPlus className="size-3.5" />
                                    </Button>
                                  </TableCell>
                                )}
                              </TableRow>
                            ))}
                          </TableBody>
                        </Table>
                      )}
                    </div>
                  )}
                </CardContent>
              </Card>
            );
          })}
        </div>
      )}

      {/* Edit Queue Dialog */}
      <Dialog open={editOpen} onOpenChange={setEditOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('queues.editQueue')}: {editQueueId}</DialogTitle>
            <DialogDescription>{t('queues.config')}</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleEdit} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="edit-max-wait">{t('queues.maxWait')}</Label>
              <Input
                id="edit-max-wait"
                type="number"
                min={0}
                value={editForm.default_max_wait_time}
                onChange={(e) => setEditForm({ ...editForm, default_max_wait_time: parseInt(e.target.value, 10) || 0 })}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="edit-max-size">{t('queues.maxSize')}</Label>
              <Input
                id="edit-max-size"
                type="number"
                min={0}
                value={editForm.max_queue_size}
                onChange={(e) => setEditForm({ ...editForm, max_queue_size: parseInt(e.target.value, 10) || 0 })}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="edit-announcement">{t('queues.announcement')}</Label>
              <Input
                id="edit-announcement"
                type="number"
                min={0}
                value={editForm.announcement_interval}
                onChange={(e) => setEditForm({ ...editForm, announcement_interval: parseInt(e.target.value, 10) || 0 })}
              />
            </div>
            <div className="flex items-center justify-between">
              <Label htmlFor="edit-priorities">{t('queues.priorities')}</Label>
              <Switch
                id="edit-priorities"
                checked={editForm.enable_priorities}
                onCheckedChange={(val) => setEditForm({ ...editForm, enable_priorities: val })}
              />
            </div>
            <div className="flex items-center justify-between">
              <Label htmlFor="edit-overflow">{t('queues.overflow')}</Label>
              <Switch
                id="edit-overflow"
                checked={editForm.enable_overflow}
                onCheckedChange={(val) => setEditForm({ ...editForm, enable_overflow: val })}
              />
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setEditOpen(false)}>
                {t('common.cancel')}
              </Button>
              <Button type="submit" disabled={updateMutation.isPending}>
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
            <DialogTitle>{t('queues.deleteQueue')}</DialogTitle>
            <DialogDescription>{t('queues.deleteConfirm')}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteOpen(false)}>
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

      {/* Assign Call Dialog */}
      <Dialog open={assignOpen} onOpenChange={setAssignOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('queues.assignCall')}</DialogTitle>
            <DialogDescription>{t('queues.assignAgent')}</DialogDescription>
          </DialogHeader>
          <div className="space-y-2">
            <Label>{t('queues.assignAgent')}</Label>
            <Select
              value={selectedAgentId || undefined}
              onValueChange={(val) => setSelectedAgentId(val as string)}
            >
              <SelectTrigger className="w-full">
                <SelectValue placeholder={t('queues.assignAgent')} />
              </SelectTrigger>
              <SelectContent>
                {agents.map((agent) => (
                  <SelectItem key={agent.id} value={agent.id}>
                    <span className="font-mono text-sm">{agent.display_name || agent.id}</span>
                    <Badge variant="secondary" className="ml-2 text-[10px]">
                      {agent.status}
                    </Badge>
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setAssignOpen(false)}>
              {t('common.cancel')}
            </Button>
            <Button
              onClick={confirmAssign}
              disabled={!selectedAgentId || assignMutation.isPending}
            >
              {t('common.confirm')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
