import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table';
import { CalendarDays, Plus, Pencil, Trash2, LogIn, LogOut, Users, UserX, Palmtree } from 'lucide-react';
import {
  fetchShifts,
  createShift,
  updateShift,
  deleteShift,
  fetchScheduleEntries,
  createScheduleEntry,
  deleteScheduleEntry,
  checkinEntry,
  checkoutEntry,
  fetchTodayAttendance,
  fetchAgents,
} from '@/lib/api';
import type { ShiftView, AgentView } from '@/lib/api';

const selectClasses =
  'flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-xs transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring';

// -- Shift form state ---------------------------------------------------------

interface ShiftFormState {
  name: string;
  start_time: string;
  end_time: string;
  break_minutes: string;
  color: string;
}

const emptyShiftForm: ShiftFormState = {
  name: '',
  start_time: '09:00',
  end_time: '17:00',
  break_minutes: '60',
  color: '#4CAF50',
};

// -- Entry form state ---------------------------------------------------------

interface EntryFormState {
  agent_id: string;
  shift_id: string;
  date: string;
}

function todayStr(): string {
  return new Date().toISOString().slice(0, 10);
}

const emptyEntryForm: EntryFormState = {
  agent_id: '',
  shift_id: '',
  date: todayStr(),
};

// -- Status badge -------------------------------------------------------------

function statusColor(status: string): 'default' | 'secondary' | 'destructive' | 'outline' {
  switch (status) {
    case 'checked_in': return 'default';
    case 'checked_out': return 'secondary';
    case 'absent': return 'destructive';
    case 'leave': return 'outline';
    default: return 'secondary';
  }
}

// -- Component ----------------------------------------------------------------

export function Schedules() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();

  // Date filter for entries
  const [selectedDate, setSelectedDate] = useState(todayStr());

  // Shift dialogs
  const [shiftCreateOpen, setShiftCreateOpen] = useState(false);
  const [shiftEditOpen, setShiftEditOpen] = useState(false);
  const [shiftDeleteOpen, setShiftDeleteOpen] = useState(false);
  const [shiftEditId, setShiftEditId] = useState('');
  const [shiftDeleteId, setShiftDeleteId] = useState('');
  const [shiftForm, setShiftForm] = useState<ShiftFormState>({ ...emptyShiftForm });

  // Entry dialogs
  const [entryCreateOpen, setEntryCreateOpen] = useState(false);
  const [entryDeleteOpen, setEntryDeleteOpen] = useState(false);
  const [entryDeleteId, setEntryDeleteId] = useState('');
  const [entryForm, setEntryForm] = useState<EntryFormState>({ ...emptyEntryForm });

  // Queries
  const { data: shifts = [] } = useQuery({ queryKey: ['shifts'], queryFn: fetchShifts });
  const { data: entries = [] } = useQuery({
    queryKey: ['schedule-entries', selectedDate],
    queryFn: () => fetchScheduleEntries({ date: selectedDate }),
  });
  const { data: attendance } = useQuery({ queryKey: ['today-attendance'], queryFn: fetchTodayAttendance });
  const { data: agentsResp } = useQuery({ queryKey: ['agents'], queryFn: fetchAgents });
  const agents: AgentView[] = agentsResp?.agents ?? [];

  // Shift mutations
  const createShiftMut = useMutation({
    mutationFn: (data: { name: string; start_time: string; end_time: string; break_minutes?: number; color?: string }) => createShift(data),
    onSuccess: () => { queryClient.invalidateQueries({ queryKey: ['shifts'] }); setShiftCreateOpen(false); setShiftForm({ ...emptyShiftForm }); },
  });

  const updateShiftMut = useMutation({
    mutationFn: ({ id, data }: { id: string; data: { name?: string; start_time?: string; end_time?: string; break_minutes?: number; color?: string } }) => updateShift(id, data),
    onSuccess: () => { queryClient.invalidateQueries({ queryKey: ['shifts'] }); setShiftEditOpen(false); setShiftForm({ ...emptyShiftForm }); },
  });

  const deleteShiftMut = useMutation({
    mutationFn: (id: string) => deleteShift(id),
    onSuccess: () => { queryClient.invalidateQueries({ queryKey: ['shifts'] }); setShiftDeleteOpen(false); },
  });

  // Entry mutations
  const createEntryMut = useMutation({
    mutationFn: (data: { agent_id: string; shift_id: string; date: string }) => createScheduleEntry(data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['schedule-entries'] });
      queryClient.invalidateQueries({ queryKey: ['today-attendance'] });
      setEntryCreateOpen(false);
      setEntryForm({ ...emptyEntryForm });
    },
  });

  const deleteEntryMut = useMutation({
    mutationFn: (id: string) => deleteScheduleEntry(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['schedule-entries'] });
      queryClient.invalidateQueries({ queryKey: ['today-attendance'] });
      setEntryDeleteOpen(false);
    },
  });

  const checkinMut = useMutation({
    mutationFn: (id: string) => checkinEntry(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['schedule-entries'] });
      queryClient.invalidateQueries({ queryKey: ['today-attendance'] });
    },
  });

  const checkoutMut = useMutation({
    mutationFn: (id: string) => checkoutEntry(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['schedule-entries'] });
      queryClient.invalidateQueries({ queryKey: ['today-attendance'] });
    },
  });

  // Helpers
  function shiftById(id: string | null): ShiftView | undefined {
    if (!id) return undefined;
    return shifts.find((s) => s.id === id);
  }

  function agentName(id: string): string {
    const a = agents.find((ag) => ag.id === id);
    return a ? a.display_name || a.id : id;
  }

  // -- Shift handlers ---------------------------------------------------------

  function handleShiftCreate(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    createShiftMut.mutate({
      name: shiftForm.name,
      start_time: shiftForm.start_time,
      end_time: shiftForm.end_time,
      break_minutes: parseInt(shiftForm.break_minutes, 10) || 60,
      color: shiftForm.color,
    });
  }

  function handleShiftUpdate(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    updateShiftMut.mutate({
      id: shiftEditId,
      data: {
        name: shiftForm.name,
        start_time: shiftForm.start_time,
        end_time: shiftForm.end_time,
        break_minutes: parseInt(shiftForm.break_minutes, 10) || 60,
        color: shiftForm.color,
      },
    });
  }

  function openShiftEdit(shift: ShiftView) {
    setShiftEditId(shift.id);
    setShiftForm({
      name: shift.name,
      start_time: shift.start_time,
      end_time: shift.end_time,
      break_minutes: String(shift.break_minutes),
      color: shift.color,
    });
    setShiftEditOpen(true);
  }

  // -- Entry handlers ---------------------------------------------------------

  function handleEntryCreate(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    createEntryMut.mutate({
      agent_id: entryForm.agent_id,
      shift_id: entryForm.shift_id,
      date: entryForm.date,
    });
  }

  // -- Render -----------------------------------------------------------------

  return (
    <div className="p-6 space-y-6">
      {/* Header */}
      <div>
        <h1 className="text-2xl font-bold flex items-center gap-2">
          <CalendarDays className="size-6" />
          {t('schedules.title')}
        </h1>
        <p className="text-muted-foreground text-sm mt-1">{t('schedules.subtitle')}</p>
      </div>

      {/* Today's Attendance Summary */}
      <div>
        <h2 className="text-lg font-semibold mb-3">{t('schedules.today')}</h2>
        <div className="grid grid-cols-2 sm:grid-cols-4 gap-4">
          <Card>
            <CardContent className="pt-4 pb-4 flex items-center gap-3">
              <Users className="size-5 text-blue-500" />
              <div>
                <div className="text-2xl font-bold">{attendance?.scheduled ?? 0}</div>
                <div className="text-xs text-muted-foreground">{t('schedules.scheduled')}</div>
              </div>
            </CardContent>
          </Card>
          <Card>
            <CardContent className="pt-4 pb-4 flex items-center gap-3">
              <LogIn className="size-5 text-green-500" />
              <div>
                <div className="text-2xl font-bold">{attendance?.checked_in ?? 0}</div>
                <div className="text-xs text-muted-foreground">{t('schedules.checkedIn')}</div>
              </div>
            </CardContent>
          </Card>
          <Card>
            <CardContent className="pt-4 pb-4 flex items-center gap-3">
              <UserX className="size-5 text-red-500" />
              <div>
                <div className="text-2xl font-bold">{attendance?.absent ?? 0}</div>
                <div className="text-xs text-muted-foreground">{t('schedules.absent')}</div>
              </div>
            </CardContent>
          </Card>
          <Card>
            <CardContent className="pt-4 pb-4 flex items-center gap-3">
              <Palmtree className="size-5 text-orange-500" />
              <div>
                <div className="text-2xl font-bold">{attendance?.leave ?? 0}</div>
                <div className="text-xs text-muted-foreground">{t('schedules.leave')}</div>
              </div>
            </CardContent>
          </Card>
        </div>
      </div>

      {/* Shift Definitions */}
      <Card>
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-4">
          <div>
            <CardTitle>{t('schedules.shifts')}</CardTitle>
            <CardDescription className="mt-1">{t('schedules.subtitle')}</CardDescription>
          </div>
          <Button size="sm" onClick={() => { setShiftForm({ ...emptyShiftForm }); setShiftCreateOpen(true); }}>
            <Plus className="size-4 mr-1" /> {t('schedules.addShift')}
          </Button>
        </CardHeader>
        <CardContent>
          {shifts.length === 0 ? (
            <p className="text-sm text-muted-foreground text-center py-8">{t('schedules.noShifts')}</p>
          ) : (
            <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
              {shifts.map((shift) => (
                <div key={shift.id} className="flex items-center gap-3 rounded-lg border p-3">
                  <div className="h-8 w-8 rounded-full shrink-0" style={{ backgroundColor: shift.color }} />
                  <div className="flex-1 min-w-0">
                    <div className="font-medium text-sm truncate">{shift.name}</div>
                    <div className="text-xs text-muted-foreground">
                      {shift.start_time} - {shift.end_time} | {shift.break_minutes} min break
                    </div>
                  </div>
                  <div className="flex gap-1">
                    <Button variant="ghost" size="icon-xs" onClick={() => openShiftEdit(shift)}>
                      <Pencil className="size-3.5" />
                    </Button>
                    <Button variant="ghost" size="icon-xs" onClick={() => { setShiftDeleteId(shift.id); setShiftDeleteOpen(true); }}>
                      <Trash2 className="size-3.5" />
                    </Button>
                  </div>
                </div>
              ))}
            </div>
          )}
        </CardContent>
      </Card>

      {/* Schedule Entries */}
      <Card>
        <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-4">
          <div>
            <CardTitle>{t('schedules.schedule')}</CardTitle>
          </div>
          <div className="flex items-center gap-2">
            <Input
              type="date"
              className="w-40"
              value={selectedDate}
              onChange={(e) => setSelectedDate(e.target.value)}
            />
            <Button size="sm" onClick={() => { setEntryForm({ ...emptyEntryForm, date: selectedDate }); setEntryCreateOpen(true); }}>
              <Plus className="size-4 mr-1" /> {t('schedules.assignShift')}
            </Button>
          </div>
        </CardHeader>
        <CardContent>
          {entries.length === 0 ? (
            <p className="text-sm text-muted-foreground text-center py-8">{t('schedules.noEntries')}</p>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t('schedules.agent')}</TableHead>
                  <TableHead>{t('schedules.shift')}</TableHead>
                  <TableHead>{t('schedules.date')}</TableHead>
                  <TableHead>{t('schedules.status')}</TableHead>
                  <TableHead>{t('schedules.checkin')}</TableHead>
                  <TableHead>{t('schedules.checkout')}</TableHead>
                  <TableHead className="text-right" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {entries.map((entry) => {
                  const shift = shiftById(entry.shift_id);
                  return (
                    <TableRow key={entry.id}>
                      <TableCell className="font-medium">{agentName(entry.agent_id)}</TableCell>
                      <TableCell>
                        {shift ? (
                          <Badge variant="secondary" className="gap-1">
                            <span className="inline-block h-2 w-2 rounded-full" style={{ backgroundColor: shift.color }} />
                            {shift.name}
                          </Badge>
                        ) : (
                          entry.shift_id ?? '-'
                        )}
                      </TableCell>
                      <TableCell>{entry.date}</TableCell>
                      <TableCell>
                        <Badge variant={statusColor(entry.status)}>{entry.status}</Badge>
                      </TableCell>
                      <TableCell className="text-xs text-muted-foreground">
                        {entry.check_in_at ? new Date(entry.check_in_at).toLocaleTimeString() : '-'}
                      </TableCell>
                      <TableCell className="text-xs text-muted-foreground">
                        {entry.check_out_at ? new Date(entry.check_out_at).toLocaleTimeString() : '-'}
                      </TableCell>
                      <TableCell className="text-right">
                        <div className="flex justify-end gap-1">
                          {entry.status === 'scheduled' && (
                            <Button variant="outline" size="sm" onClick={() => checkinMut.mutate(entry.id)}>
                              <LogIn className="size-3.5 mr-1" /> {t('schedules.checkin')}
                            </Button>
                          )}
                          {entry.status === 'checked_in' && (
                            <Button variant="outline" size="sm" onClick={() => checkoutMut.mutate(entry.id)}>
                              <LogOut className="size-3.5 mr-1" /> {t('schedules.checkout')}
                            </Button>
                          )}
                          <Button variant="ghost" size="icon-xs" onClick={() => { setEntryDeleteId(entry.id); setEntryDeleteOpen(true); }}>
                            <Trash2 className="size-3.5" />
                          </Button>
                        </div>
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>

      {/* Create Shift Dialog */}
      <Dialog open={shiftCreateOpen} onOpenChange={setShiftCreateOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('schedules.addShift')}</DialogTitle>
            <DialogDescription>{t('schedules.subtitle')}</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleShiftCreate} className="space-y-4">
            <div className="space-y-2">
              <Label>{t('schedules.shiftName')}</Label>
              <Input value={shiftForm.name} onChange={(e) => setShiftForm({ ...shiftForm, name: e.target.value })} required />
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label>{t('schedules.startTime')}</Label>
                <Input type="time" value={shiftForm.start_time} onChange={(e) => setShiftForm({ ...shiftForm, start_time: e.target.value })} required />
              </div>
              <div className="space-y-2">
                <Label>{t('schedules.endTime')}</Label>
                <Input type="time" value={shiftForm.end_time} onChange={(e) => setShiftForm({ ...shiftForm, end_time: e.target.value })} required />
              </div>
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label>{t('schedules.breakMinutes')}</Label>
                <Input type="number" value={shiftForm.break_minutes} onChange={(e) => setShiftForm({ ...shiftForm, break_minutes: e.target.value })} />
              </div>
              <div className="space-y-2">
                <Label>{t('schedules.color')}</Label>
                <Input type="color" value={shiftForm.color} onChange={(e) => setShiftForm({ ...shiftForm, color: e.target.value })} className="h-9 p-1" />
              </div>
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setShiftCreateOpen(false)}>{t('common.cancel')}</Button>
              <Button type="submit">{t('common.save')}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* Edit Shift Dialog */}
      <Dialog open={shiftEditOpen} onOpenChange={setShiftEditOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('schedules.editShift')}</DialogTitle>
            <DialogDescription>{t('schedules.subtitle')}</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleShiftUpdate} className="space-y-4">
            <div className="space-y-2">
              <Label>{t('schedules.shiftName')}</Label>
              <Input value={shiftForm.name} onChange={(e) => setShiftForm({ ...shiftForm, name: e.target.value })} required />
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label>{t('schedules.startTime')}</Label>
                <Input type="time" value={shiftForm.start_time} onChange={(e) => setShiftForm({ ...shiftForm, start_time: e.target.value })} required />
              </div>
              <div className="space-y-2">
                <Label>{t('schedules.endTime')}</Label>
                <Input type="time" value={shiftForm.end_time} onChange={(e) => setShiftForm({ ...shiftForm, end_time: e.target.value })} required />
              </div>
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label>{t('schedules.breakMinutes')}</Label>
                <Input type="number" value={shiftForm.break_minutes} onChange={(e) => setShiftForm({ ...shiftForm, break_minutes: e.target.value })} />
              </div>
              <div className="space-y-2">
                <Label>{t('schedules.color')}</Label>
                <Input type="color" value={shiftForm.color} onChange={(e) => setShiftForm({ ...shiftForm, color: e.target.value })} className="h-9 p-1" />
              </div>
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setShiftEditOpen(false)}>{t('common.cancel')}</Button>
              <Button type="submit">{t('common.save')}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* Delete Shift Confirmation */}
      <Dialog open={shiftDeleteOpen} onOpenChange={setShiftDeleteOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('common.delete')}</DialogTitle>
            <DialogDescription>{t('schedules.deleteConfirm')}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setShiftDeleteOpen(false)}>{t('common.cancel')}</Button>
            <Button variant="destructive" onClick={() => deleteShiftMut.mutate(shiftDeleteId)}>{t('common.delete')}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Create Entry Dialog */}
      <Dialog open={entryCreateOpen} onOpenChange={setEntryCreateOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('schedules.assignShift')}</DialogTitle>
            <DialogDescription>{t('schedules.subtitle')}</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleEntryCreate} className="space-y-4">
            <div className="space-y-2">
              <Label>{t('schedules.agent')}</Label>
              <select
                className={selectClasses}
                value={entryForm.agent_id}
                onChange={(e) => setEntryForm({ ...entryForm, agent_id: e.target.value })}
                required
              >
                <option value="">--</option>
                {agents.map((a) => (
                  <option key={a.id} value={a.id}>{a.display_name || a.id}</option>
                ))}
              </select>
            </div>
            <div className="space-y-2">
              <Label>{t('schedules.shift')}</Label>
              <select
                className={selectClasses}
                value={entryForm.shift_id}
                onChange={(e) => setEntryForm({ ...entryForm, shift_id: e.target.value })}
                required
              >
                <option value="">--</option>
                {shifts.map((s) => (
                  <option key={s.id} value={s.id}>{s.name} ({s.start_time}-{s.end_time})</option>
                ))}
              </select>
            </div>
            <div className="space-y-2">
              <Label>{t('schedules.date')}</Label>
              <Input type="date" value={entryForm.date} onChange={(e) => setEntryForm({ ...entryForm, date: e.target.value })} required />
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setEntryCreateOpen(false)}>{t('common.cancel')}</Button>
              <Button type="submit">{t('common.save')}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* Delete Entry Confirmation */}
      <Dialog open={entryDeleteOpen} onOpenChange={setEntryDeleteOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('common.delete')}</DialogTitle>
            <DialogDescription>{t('schedules.deleteConfirm')}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setEntryDeleteOpen(false)}>{t('common.cancel')}</Button>
            <Button variant="destructive" onClick={() => deleteEntryMut.mutate(entryDeleteId)}>{t('common.delete')}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
