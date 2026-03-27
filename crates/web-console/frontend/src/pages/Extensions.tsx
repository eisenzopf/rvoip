import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
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
import { Plus, Trash2, Phone } from 'lucide-react';
import {
  fetchExtensionRanges,
  createExtensionRange,
  deleteExtensionRange,
} from '@/lib/api';
import type { ExtensionRangeView } from '@/lib/api';

interface RangeFormState {
  range_start: string;
  range_end: string;
  department_id: string;
  description: string;
}

const defaultForm: RangeFormState = {
  range_start: '',
  range_end: '',
  department_id: '',
  description: '',
};

export function Extensions() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();

  const [createOpen, setCreateOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [deleteId, setDeleteId] = useState('');
  const [form, setForm] = useState<RangeFormState>({ ...defaultForm });

  const { data: ranges } = useQuery<ExtensionRangeView[]>({
    queryKey: ['extensionRanges'],
    queryFn: fetchExtensionRanges,
  });

  const createMut = useMutation({
    mutationFn: (data: { range_start: number; range_end: number; department_id?: string; description?: string }) =>
      createExtensionRange(data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['extensionRanges'] });
      setCreateOpen(false);
      setForm({ ...defaultForm });
    },
  });

  const deleteMut = useMutation({
    mutationFn: (id: string) => deleteExtensionRange(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['extensionRanges'] });
      setDeleteOpen(false);
    },
  });

  const handleCreate = (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const start = parseInt(form.range_start, 10);
    const end = parseInt(form.range_end, 10);
    if (isNaN(start) || isNaN(end)) return;
    createMut.mutate({
      range_start: start,
      range_end: end,
      department_id: form.department_id || undefined,
      description: form.description || undefined,
    });
  };

  const totalAll = (ranges ?? []).reduce((s, r) => s + r.total, 0);
  const assignedAll = (ranges ?? []).reduce((s, r) => s + r.assigned, 0);
  const availableAll = (ranges ?? []).reduce((s, r) => s + r.available, 0);

  return (
    <div className="p-6 space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">{t('extensions.title')}</h1>
          <p className="text-muted-foreground text-sm">{t('extensions.ranges')}</p>
        </div>
        <Dialog open={createOpen} onOpenChange={setCreateOpen}>
          <DialogTrigger render={<Button size="sm" />}>
            <Plus className="size-4 mr-1" />{t('extensions.addRange')}
          </DialogTrigger>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>{t('extensions.addRange')}</DialogTitle>
              <DialogDescription>{t('extensions.ranges')}</DialogDescription>
            </DialogHeader>
            <form onSubmit={handleCreate} className="space-y-4">
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <Label>{t('extensions.rangeStart')}</Label>
                  <Input type="number" value={form.range_start} onChange={(e) => setForm({ ...form, range_start: e.target.value })} required />
                </div>
                <div>
                  <Label>{t('extensions.rangeEnd')}</Label>
                  <Input type="number" value={form.range_end} onChange={(e) => setForm({ ...form, range_end: e.target.value })} required />
                </div>
              </div>
              <div>
                <Label>{t('extensions.department')}</Label>
                <Input value={form.department_id} onChange={(e) => setForm({ ...form, department_id: e.target.value })} />
              </div>
              <div>
                <Label>{t('extensions.description')}</Label>
                <Input value={form.description} onChange={(e) => setForm({ ...form, description: e.target.value })} />
              </div>
              <DialogFooter>
                <Button type="button" variant="outline" onClick={() => setCreateOpen(false)}>{t('common.cancel')}</Button>
                <Button type="submit">{t('common.save')}</Button>
              </DialogFooter>
            </form>
          </DialogContent>
        </Dialog>
      </div>

      {/* Summary cards */}
      <div className="grid grid-cols-3 gap-4">
        <Card>
          <CardHeader className="pb-2"><CardTitle className="text-sm text-muted-foreground">{t('extensions.total')}</CardTitle></CardHeader>
          <CardContent><p className="text-2xl font-bold">{totalAll}</p></CardContent>
        </Card>
        <Card>
          <CardHeader className="pb-2"><CardTitle className="text-sm text-muted-foreground">{t('extensions.assigned')}</CardTitle></CardHeader>
          <CardContent><p className="text-2xl font-bold">{assignedAll}</p></CardContent>
        </Card>
        <Card>
          <CardHeader className="pb-2"><CardTitle className="text-sm text-muted-foreground">{t('extensions.available')}</CardTitle></CardHeader>
          <CardContent><p className="text-2xl font-bold text-green-600">{availableAll}</p></CardContent>
        </Card>
      </div>

      {/* Ranges table */}
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2"><Phone className="size-4" />{t('extensions.ranges')}</CardTitle>
        </CardHeader>
        <CardContent>
          {(!ranges || ranges.length === 0) ? (
            <p className="text-muted-foreground text-center py-8">{t('extensions.noRanges')}</p>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>ID</TableHead>
                  <TableHead>{t('extensions.rangeStart')} - {t('extensions.rangeEnd')}</TableHead>
                  <TableHead>{t('extensions.department')}</TableHead>
                  <TableHead>{t('extensions.description')}</TableHead>
                  <TableHead>{t('extensions.total')}</TableHead>
                  <TableHead>{t('extensions.assigned')}</TableHead>
                  <TableHead>{t('extensions.available')}</TableHead>
                  <TableHead></TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {ranges.map((r) => {
                  const pct = r.total > 0 ? Math.round((r.assigned / r.total) * 100) : 0;
                  return (
                    <TableRow key={r.id}>
                      <TableCell className="font-mono text-xs">{r.id}</TableCell>
                      <TableCell>{r.range_start} - {r.range_end}</TableCell>
                      <TableCell>{r.department_id ?? '-'}</TableCell>
                      <TableCell className="text-muted-foreground">{r.description ?? '-'}</TableCell>
                      <TableCell>{r.total}</TableCell>
                      <TableCell>{r.assigned}</TableCell>
                      <TableCell>
                        <div className="flex items-center gap-2">
                          <span className="text-green-600">{r.available}</span>
                          <div className="w-16 h-2 rounded bg-muted overflow-hidden">
                            <div className="h-full bg-primary rounded" style={{ width: `${pct}%` }} />
                          </div>
                        </div>
                      </TableCell>
                      <TableCell>
                        <Button
                          variant="ghost"
                          size="icon-xs"
                          onClick={() => { setDeleteId(r.id); setDeleteOpen(true); }}
                        >
                          <Trash2 className="size-3.5 text-destructive" />
                        </Button>
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>

      {/* Delete confirmation dialog */}
      <Dialog open={deleteOpen} onOpenChange={setDeleteOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('common.confirm')}</DialogTitle>
            <DialogDescription>{t('extensions.deleteConfirm')}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteOpen(false)}>{t('common.cancel')}</Button>
            <Button variant="destructive" onClick={() => deleteMut.mutate(deleteId)}>{t('common.delete')}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
