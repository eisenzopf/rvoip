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
import { Plus, Pencil, Trash2, Cable, Phone } from 'lucide-react';
import {
  fetchTrunks,
  createTrunk,
  updateTrunk,
  deleteTrunk,
  fetchDids,
  createDid,
  updateDid,
  deleteDid,
} from '@/lib/api';
import type { TrunkView, DidNumberView } from '@/lib/api';

// --- Trunk form state ---
interface TrunkFormState {
  name: string;
  provider: string;
  host: string;
  port: string;
  transport: string;
  username: string;
  password: string;
  max_channels: string;
  registration_required: boolean;
}

const defaultTrunkForm: TrunkFormState = {
  name: '',
  provider: '',
  host: '',
  port: '5060',
  transport: 'UDP',
  username: '',
  password: '',
  max_channels: '30',
  registration_required: false,
};

// --- DID form state ---
interface DidFormState {
  number: string;
  trunk_id: string;
  assigned_to: string;
  assigned_type: string;
  description: string;
}

const defaultDidForm: DidFormState = {
  number: '',
  trunk_id: '',
  assigned_to: '',
  assigned_type: 'unassigned',
  description: '',
};

function statusColor(status: string): string {
  if (status === 'active') return 'bg-green-500';
  if (status === 'inactive') return 'bg-yellow-500';
  return 'bg-red-500';
}

function assignedTypeBadgeVariant(at: string | null): 'default' | 'secondary' | 'outline' | 'destructive' {
  if (at === 'queue') return 'default';
  if (at === 'agent') return 'secondary';
  if (at === 'ivr') return 'outline';
  return 'outline';
}

export function Trunks() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();

  // Trunk dialog state
  const [createTrunkOpen, setCreateTrunkOpen] = useState(false);
  const [editTrunkOpen, setEditTrunkOpen] = useState(false);
  const [deleteTrunkOpen, setDeleteTrunkOpen] = useState(false);
  const [editTrunkId, setEditTrunkId] = useState('');
  const [deleteTrunkId, setDeleteTrunkId] = useState('');
  const [trunkForm, setTrunkForm] = useState<TrunkFormState>({ ...defaultTrunkForm });

  // DID dialog state
  const [createDidOpen, setCreateDidOpen] = useState(false);
  const [editDidOpen, setEditDidOpen] = useState(false);
  const [deleteDidOpen, setDeleteDidOpen] = useState(false);
  const [editDidId, setEditDidId] = useState('');
  const [deleteDidId, setDeleteDidId] = useState('');
  const [didForm, setDidForm] = useState<DidFormState>({ ...defaultDidForm });

  // Queries
  const { data: trunks } = useQuery<TrunkView[]>({
    queryKey: ['trunks'],
    queryFn: fetchTrunks,
  });

  const { data: dids } = useQuery<DidNumberView[]>({
    queryKey: ['dids'],
    queryFn: fetchDids,
  });

  // --- Trunk mutations ---
  const createTrunkMut = useMutation({
    mutationFn: (data: Parameters<typeof createTrunk>[0]) => createTrunk(data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['trunks'] });
      setCreateTrunkOpen(false);
      setTrunkForm({ ...defaultTrunkForm });
    },
  });

  const updateTrunkMut = useMutation({
    mutationFn: ({ id, data }: { id: string; data: Parameters<typeof updateTrunk>[1] }) => updateTrunk(id, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['trunks'] });
      setEditTrunkOpen(false);
      setTrunkForm({ ...defaultTrunkForm });
    },
  });

  const deleteTrunkMut = useMutation({
    mutationFn: (id: string) => deleteTrunk(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['trunks'] });
      setDeleteTrunkOpen(false);
    },
  });

  // --- DID mutations ---
  const createDidMut = useMutation({
    mutationFn: (data: Parameters<typeof createDid>[0]) => createDid(data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['dids'] });
      setCreateDidOpen(false);
      setDidForm({ ...defaultDidForm });
    },
  });

  const updateDidMut = useMutation({
    mutationFn: ({ id, data }: { id: string; data: Parameters<typeof updateDid>[1] }) => updateDid(id, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['dids'] });
      setEditDidOpen(false);
      setDidForm({ ...defaultDidForm });
    },
  });

  const deleteDidMut = useMutation({
    mutationFn: (id: string) => deleteDid(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['dids'] });
      setDeleteDidOpen(false);
    },
  });

  // --- Trunk handlers ---
  const handleCreateTrunk = (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    if (!trunkForm.name.trim() || !trunkForm.host.trim()) return;
    createTrunkMut.mutate({
      name: trunkForm.name,
      provider: trunkForm.provider || undefined,
      host: trunkForm.host,
      port: parseInt(trunkForm.port, 10) || 5060,
      transport: trunkForm.transport || 'UDP',
      username: trunkForm.username || undefined,
      password: trunkForm.password || undefined,
      max_channels: parseInt(trunkForm.max_channels, 10) || 30,
      registration_required: trunkForm.registration_required,
    });
  };

  const handleUpdateTrunk = (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    updateTrunkMut.mutate({
      id: editTrunkId,
      data: {
        name: trunkForm.name || undefined,
        provider: trunkForm.provider || undefined,
        host: trunkForm.host || undefined,
        port: parseInt(trunkForm.port, 10) || undefined,
        transport: trunkForm.transport || undefined,
        username: trunkForm.username || undefined,
        password: trunkForm.password || undefined,
        max_channels: parseInt(trunkForm.max_channels, 10) || undefined,
        registration_required: trunkForm.registration_required,
      },
    });
  };

  const openEditTrunk = (trunk: TrunkView) => {
    setEditTrunkId(trunk.id);
    setTrunkForm({
      name: trunk.name,
      provider: trunk.provider ?? '',
      host: trunk.host,
      port: String(trunk.port),
      transport: trunk.transport,
      username: trunk.username ?? '',
      password: '',
      max_channels: String(trunk.max_channels),
      registration_required: trunk.registration_required,
    });
    setEditTrunkOpen(true);
  };

  // --- DID handlers ---
  const handleCreateDid = (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    if (!didForm.number.trim()) return;
    createDidMut.mutate({
      number: didForm.number,
      trunk_id: didForm.trunk_id || undefined,
      assigned_to: didForm.assigned_to || undefined,
      assigned_type: didForm.assigned_type || 'unassigned',
      description: didForm.description || undefined,
    });
  };

  const handleUpdateDid = (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    updateDidMut.mutate({
      id: editDidId,
      data: {
        trunk_id: didForm.trunk_id || undefined,
        assigned_to: didForm.assigned_to || undefined,
        assigned_type: didForm.assigned_type || undefined,
        description: didForm.description || undefined,
      },
    });
  };

  const openEditDid = (did: DidNumberView) => {
    setEditDidId(did.id);
    setDidForm({
      number: did.number,
      trunk_id: did.trunk_id ?? '',
      assigned_to: did.assigned_to ?? '',
      assigned_type: did.assigned_type ?? 'unassigned',
      description: did.description ?? '',
    });
    setEditDidOpen(true);
  };

  // Channel usage percentage
  const channelPercent = (active: number, max: number) => {
    if (max <= 0) return 0;
    return Math.min(100, Math.round((active / max) * 100));
  };

  return (
    <div className="p-6 space-y-8">
      {/* ==================== TRUNK SECTION ==================== */}
      <div>
        <div className="flex items-center justify-between mb-4">
          <div>
            <h1 className="text-2xl font-bold">{t('trunks.title')}</h1>
            <p className="text-muted-foreground text-sm">{t('trunks.subtitle')}</p>
          </div>
          <Dialog open={createTrunkOpen} onOpenChange={setCreateTrunkOpen}>
            <DialogTrigger render={<Button size="sm" />}>
              <Plus className="size-4 mr-1" />{t('trunks.addTrunk')}
            </DialogTrigger>
            <DialogContent>
              <DialogHeader>
                <DialogTitle>{t('trunks.addTrunk')}</DialogTitle>
                <DialogDescription>{t('trunks.subtitle')}</DialogDescription>
              </DialogHeader>
              <form onSubmit={handleCreateTrunk} className="space-y-4">
                <div className="grid grid-cols-2 gap-4">
                  <div>
                    <Label>{t('trunks.name')}</Label>
                    <Input value={trunkForm.name} onChange={(e) => setTrunkForm({ ...trunkForm, name: e.target.value })} required />
                  </div>
                  <div>
                    <Label>{t('trunks.provider')}</Label>
                    <Input value={trunkForm.provider} onChange={(e) => setTrunkForm({ ...trunkForm, provider: e.target.value })} />
                  </div>
                </div>
                <div className="grid grid-cols-3 gap-4">
                  <div>
                    <Label>{t('trunks.host')}</Label>
                    <Input value={trunkForm.host} onChange={(e) => setTrunkForm({ ...trunkForm, host: e.target.value })} required />
                  </div>
                  <div>
                    <Label>{t('trunks.port')}</Label>
                    <Input type="number" value={trunkForm.port} onChange={(e) => setTrunkForm({ ...trunkForm, port: e.target.value })} />
                  </div>
                  <div>
                    <Label>{t('trunks.transport')}</Label>
                    <Select value={trunkForm.transport} onValueChange={(val) => setTrunkForm({ ...trunkForm, transport: val as string })}>
                      <SelectTrigger className="w-full">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="UDP">UDP</SelectItem>
                        <SelectItem value="TCP">TCP</SelectItem>
                        <SelectItem value="TLS">TLS</SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                </div>
                <div className="grid grid-cols-2 gap-4">
                  <div>
                    <Label>{t('trunks.username')}</Label>
                    <Input value={trunkForm.username} onChange={(e) => setTrunkForm({ ...trunkForm, username: e.target.value })} />
                  </div>
                  <div>
                    <Label>{t('trunks.password')}</Label>
                    <Input type="password" value={trunkForm.password} onChange={(e) => setTrunkForm({ ...trunkForm, password: e.target.value })} />
                  </div>
                </div>
                <div className="grid grid-cols-2 gap-4">
                  <div>
                    <Label>{t('trunks.maxChannels')}</Label>
                    <Input type="number" value={trunkForm.max_channels} onChange={(e) => setTrunkForm({ ...trunkForm, max_channels: e.target.value })} />
                  </div>
                  <div className="flex items-center justify-between pt-6">
                    <Label htmlFor="create-reg">{t('trunks.registration')}</Label>
                    <Switch id="create-reg" checked={trunkForm.registration_required} onCheckedChange={(val) => setTrunkForm({ ...trunkForm, registration_required: val })} />
                  </div>
                </div>
                <DialogFooter>
                  <Button type="button" variant="outline" onClick={() => setCreateTrunkOpen(false)}>{t('common.cancel')}</Button>
                  <Button type="submit">{t('common.save')}</Button>
                </DialogFooter>
              </form>
            </DialogContent>
          </Dialog>
        </div>

        {/* Trunk cards */}
        {(!trunks || trunks.length === 0) ? (
          <Card>
            <CardContent className="py-12 text-center text-muted-foreground">{t('trunks.noTrunks')}</CardContent>
          </Card>
        ) : (
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
            {trunks.map((trunk) => {
              const pct = channelPercent(trunk.active_channels, trunk.max_channels);
              return (
                <Card key={trunk.id}>
                  <CardHeader className="pb-2">
                    <div className="flex items-start justify-between">
                      <div className="flex items-center gap-2">
                        <Cable className="size-4 text-primary" />
                        <CardTitle className="text-base">{trunk.name}</CardTitle>
                      </div>
                      <div className="flex items-center gap-2">
                        <div className={`h-2.5 w-2.5 rounded-full ${statusColor(trunk.status)}`} />
                        <div className="flex gap-1">
                          <Button variant="ghost" size="icon-xs" onClick={() => openEditTrunk(trunk)}>
                            <Pencil className="size-3.5" />
                          </Button>
                          <Button variant="ghost" size="icon-xs" onClick={() => { setDeleteTrunkId(trunk.id); setDeleteTrunkOpen(true); }}>
                            <Trash2 className="size-3.5 text-destructive" />
                          </Button>
                        </div>
                      </div>
                    </div>
                    {trunk.provider && (
                      <Badge variant="secondary" className="w-fit text-xs">{trunk.provider}</Badge>
                    )}
                  </CardHeader>
                  <CardContent className="space-y-3">
                    <div className="text-sm text-muted-foreground">
                      {trunk.host}:{trunk.port} <Badge variant="outline" className="ml-1 text-[10px]">{trunk.transport}</Badge>
                    </div>

                    {/* Channels bar */}
                    <div>
                      <div className="flex items-center justify-between text-xs text-muted-foreground mb-1">
                        <span>{t('trunks.channels')}</span>
                        <span>{trunk.active_channels} / {trunk.max_channels}</span>
                      </div>
                      <div className="h-2 w-full rounded-full bg-muted">
                        <div
                          className="h-2 rounded-full bg-primary transition-all"
                          style={{ width: `${pct}%` }}
                        />
                      </div>
                    </div>

                    <div className="flex items-center gap-2 text-sm text-muted-foreground">
                      <Phone className="size-3.5" />
                      <span>DID: {trunk.did_count}</span>
                    </div>
                  </CardContent>
                </Card>
              );
            })}
          </div>
        )}
      </div>

      {/* ==================== DID SECTION ==================== */}
      <div>
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-xl font-bold">{t('trunks.did')}</h2>
          <Dialog open={createDidOpen} onOpenChange={setCreateDidOpen}>
            <DialogTrigger render={<Button size="sm" />}>
              <Plus className="size-4 mr-1" />{t('trunks.addDid')}
            </DialogTrigger>
            <DialogContent>
              <DialogHeader>
                <DialogTitle>{t('trunks.addDid')}</DialogTitle>
                <DialogDescription>{t('trunks.did')}</DialogDescription>
              </DialogHeader>
              <form onSubmit={handleCreateDid} className="space-y-4">
                <div>
                  <Label>{t('trunks.didNumber')}</Label>
                  <Input value={didForm.number} onChange={(e) => setDidForm({ ...didForm, number: e.target.value })} required placeholder="+1-800-555-0001" />
                </div>
                <div>
                  <Label>{t('trunks.trunk')}</Label>
                  <Select value={didForm.trunk_id || undefined} onValueChange={(val) => setDidForm({ ...didForm, trunk_id: val as string })}>
                    <SelectTrigger className="w-full">
                      <SelectValue placeholder={t('trunks.trunk')} />
                    </SelectTrigger>
                    <SelectContent>
                      {(trunks ?? []).map((tr) => (
                        <SelectItem key={tr.id} value={tr.id}>{tr.name}</SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div className="grid grid-cols-2 gap-4">
                  <div>
                    <Label>{t('trunks.assignedType')}</Label>
                    <Select value={didForm.assigned_type} onValueChange={(val) => setDidForm({ ...didForm, assigned_type: val as string })}>
                      <SelectTrigger className="w-full">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="unassigned">unassigned</SelectItem>
                        <SelectItem value="queue">queue</SelectItem>
                        <SelectItem value="agent">agent</SelectItem>
                        <SelectItem value="ivr">ivr</SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                  <div>
                    <Label>{t('trunks.assignedTo')}</Label>
                    <Input value={didForm.assigned_to} onChange={(e) => setDidForm({ ...didForm, assigned_to: e.target.value })} />
                  </div>
                </div>
                <div>
                  <Label>{t('trunks.description')}</Label>
                  <Input value={didForm.description} onChange={(e) => setDidForm({ ...didForm, description: e.target.value })} />
                </div>
                <DialogFooter>
                  <Button type="button" variant="outline" onClick={() => setCreateDidOpen(false)}>{t('common.cancel')}</Button>
                  <Button type="submit">{t('common.save')}</Button>
                </DialogFooter>
              </form>
            </DialogContent>
          </Dialog>
        </div>

        {/* DID table */}
        {(!dids || dids.length === 0) ? (
          <Card>
            <CardContent className="py-12 text-center text-muted-foreground">{t('trunks.noDid')}</CardContent>
          </Card>
        ) : (
          <Card>
            <CardContent className="p-0">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>{t('trunks.didNumber')}</TableHead>
                    <TableHead>{t('trunks.trunk')}</TableHead>
                    <TableHead>{t('trunks.assignedType')}</TableHead>
                    <TableHead>{t('trunks.assignedTo')}</TableHead>
                    <TableHead>{t('trunks.description')}</TableHead>
                    <TableHead className="w-[80px]" />
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {dids.map((did) => (
                    <TableRow key={did.id}>
                      <TableCell className="font-mono">{did.number}</TableCell>
                      <TableCell>
                        {did.trunk_name ? (
                          <Badge variant="secondary">{did.trunk_name}</Badge>
                        ) : (
                          <span className="text-muted-foreground">-</span>
                        )}
                      </TableCell>
                      <TableCell>
                        {did.assigned_type ? (
                          <Badge variant={assignedTypeBadgeVariant(did.assigned_type)}>{did.assigned_type}</Badge>
                        ) : (
                          <span className="text-muted-foreground">-</span>
                        )}
                      </TableCell>
                      <TableCell>{did.assigned_to ?? '-'}</TableCell>
                      <TableCell className="text-muted-foreground text-sm">{did.description ?? '-'}</TableCell>
                      <TableCell>
                        <div className="flex gap-1">
                          <Button variant="ghost" size="icon-xs" onClick={() => openEditDid(did)}>
                            <Pencil className="size-3.5" />
                          </Button>
                          <Button variant="ghost" size="icon-xs" onClick={() => { setDeleteDidId(did.id); setDeleteDidOpen(true); }}>
                            <Trash2 className="size-3.5 text-destructive" />
                          </Button>
                        </div>
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </CardContent>
          </Card>
        )}
      </div>

      {/* ==================== EDIT TRUNK DIALOG ==================== */}
      <Dialog open={editTrunkOpen} onOpenChange={setEditTrunkOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('trunks.editTrunk')}</DialogTitle>
            <DialogDescription>{editTrunkId}</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleUpdateTrunk} className="space-y-4">
            <div className="grid grid-cols-2 gap-4">
              <div>
                <Label>{t('trunks.name')}</Label>
                <Input value={trunkForm.name} onChange={(e) => setTrunkForm({ ...trunkForm, name: e.target.value })} required />
              </div>
              <div>
                <Label>{t('trunks.provider')}</Label>
                <Input value={trunkForm.provider} onChange={(e) => setTrunkForm({ ...trunkForm, provider: e.target.value })} />
              </div>
            </div>
            <div className="grid grid-cols-3 gap-4">
              <div>
                <Label>{t('trunks.host')}</Label>
                <Input value={trunkForm.host} onChange={(e) => setTrunkForm({ ...trunkForm, host: e.target.value })} required />
              </div>
              <div>
                <Label>{t('trunks.port')}</Label>
                <Input type="number" value={trunkForm.port} onChange={(e) => setTrunkForm({ ...trunkForm, port: e.target.value })} />
              </div>
              <div>
                <Label>{t('trunks.transport')}</Label>
                <Select value={trunkForm.transport} onValueChange={(val) => setTrunkForm({ ...trunkForm, transport: val as string })}>
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="UDP">UDP</SelectItem>
                    <SelectItem value="TCP">TCP</SelectItem>
                    <SelectItem value="TLS">TLS</SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div>
                <Label>{t('trunks.username')}</Label>
                <Input value={trunkForm.username} onChange={(e) => setTrunkForm({ ...trunkForm, username: e.target.value })} />
              </div>
              <div>
                <Label>{t('trunks.password')}</Label>
                <Input type="password" value={trunkForm.password} onChange={(e) => setTrunkForm({ ...trunkForm, password: e.target.value })} placeholder="(unchanged)" />
              </div>
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div>
                <Label>{t('trunks.maxChannels')}</Label>
                <Input type="number" value={trunkForm.max_channels} onChange={(e) => setTrunkForm({ ...trunkForm, max_channels: e.target.value })} />
              </div>
              <div className="flex items-center justify-between pt-6">
                <Label htmlFor="edit-reg">{t('trunks.registration')}</Label>
                <Switch id="edit-reg" checked={trunkForm.registration_required} onCheckedChange={(val) => setTrunkForm({ ...trunkForm, registration_required: val })} />
              </div>
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setEditTrunkOpen(false)}>{t('common.cancel')}</Button>
              <Button type="submit">{t('common.save')}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* ==================== DELETE TRUNK DIALOG ==================== */}
      <Dialog open={deleteTrunkOpen} onOpenChange={setDeleteTrunkOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('common.confirm')}</DialogTitle>
            <DialogDescription>{t('trunks.deleteConfirm')}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteTrunkOpen(false)}>{t('common.cancel')}</Button>
            <Button variant="destructive" onClick={() => deleteTrunkMut.mutate(deleteTrunkId)}>{t('common.delete')}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* ==================== EDIT DID DIALOG ==================== */}
      <Dialog open={editDidOpen} onOpenChange={setEditDidOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('trunks.editDid')}</DialogTitle>
            <DialogDescription>{editDidId}</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleUpdateDid} className="space-y-4">
            <div>
              <Label>{t('trunks.trunk')}</Label>
              <Select value={didForm.trunk_id || undefined} onValueChange={(val) => setDidForm({ ...didForm, trunk_id: val as string })}>
                <SelectTrigger className="w-full">
                  <SelectValue placeholder={t('trunks.trunk')} />
                </SelectTrigger>
                <SelectContent>
                  {(trunks ?? []).map((tr) => (
                    <SelectItem key={tr.id} value={tr.id}>{tr.name}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div>
                <Label>{t('trunks.assignedType')}</Label>
                <Select value={didForm.assigned_type} onValueChange={(val) => setDidForm({ ...didForm, assigned_type: val as string })}>
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="unassigned">unassigned</SelectItem>
                    <SelectItem value="queue">queue</SelectItem>
                    <SelectItem value="agent">agent</SelectItem>
                    <SelectItem value="ivr">ivr</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div>
                <Label>{t('trunks.assignedTo')}</Label>
                <Input value={didForm.assigned_to} onChange={(e) => setDidForm({ ...didForm, assigned_to: e.target.value })} />
              </div>
            </div>
            <div>
              <Label>{t('trunks.description')}</Label>
              <Input value={didForm.description} onChange={(e) => setDidForm({ ...didForm, description: e.target.value })} />
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setEditDidOpen(false)}>{t('common.cancel')}</Button>
              <Button type="submit">{t('common.save')}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* ==================== DELETE DID DIALOG ==================== */}
      <Dialog open={deleteDidOpen} onOpenChange={setDeleteDidOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('common.confirm')}</DialogTitle>
            <DialogDescription>{t('trunks.deleteConfirm')}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteDidOpen(false)}>{t('common.cancel')}</Button>
            <Button variant="destructive" onClick={() => deleteDidMut.mutate(deleteDidId)}>{t('common.delete')}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
