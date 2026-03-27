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
import { Building2, Plus, Pencil, Trash2, Users } from 'lucide-react';
import {
  fetchDepartments,
  createDepartment,
  updateDepartment,
  deleteDepartment,
} from '@/lib/api';
import type { DepartmentView } from '@/lib/api';

const selectClasses =
  'flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-xs transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring';

interface DeptFormState {
  name: string;
  description: string;
  parent_id: string;
}

const emptyForm: DeptFormState = { name: '', description: '', parent_id: '' };

export function Departments() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();

  const [createOpen, setCreateOpen] = useState(false);
  const [editOpen, setEditOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [editId, setEditId] = useState('');
  const [deleteId, setDeleteId] = useState('');
  const [form, setForm] = useState<DeptFormState>({ ...emptyForm });

  const { data: departments = [] } = useQuery({
    queryKey: ['departments'],
    queryFn: fetchDepartments,
  });

  const createMut = useMutation({
    mutationFn: (data: { name: string; description?: string; parent_id?: string }) =>
      createDepartment(data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['departments'] });
      setCreateOpen(false);
      setForm({ ...emptyForm });
    },
  });

  const updateMut = useMutation({
    mutationFn: ({ id, data }: { id: string; data: { name?: string; description?: string; parent_id?: string } }) =>
      updateDepartment(id, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['departments'] });
      setEditOpen(false);
      setForm({ ...emptyForm });
    },
  });

  const deleteMut = useMutation({
    mutationFn: (id: string) => deleteDepartment(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['departments'] });
      setDeleteOpen(false);
    },
  });

  function handleCreate(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    createMut.mutate({
      name: form.name,
      description: form.description || undefined,
      parent_id: form.parent_id || undefined,
    });
  }

  function handleEdit(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    updateMut.mutate({
      id: editId,
      data: {
        name: form.name || undefined,
        description: form.description || undefined,
        parent_id: form.parent_id || undefined,
      },
    });
  }

  function openEdit(dept: DepartmentView) {
    setEditId(dept.id);
    setForm({
      name: dept.name,
      description: dept.description ?? '',
      parent_id: dept.parent_id ?? '',
    });
    setEditOpen(true);
  }

  function openDelete(id: string) {
    setDeleteId(id);
    setDeleteOpen(true);
  }

  function parentName(parentId: string | null): string | null {
    if (!parentId) return null;
    const p = departments.find((d) => d.id === parentId);
    return p ? p.name : parentId;
  }

  return (
    <div className="p-6 space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
            <Building2 className="size-6" />
            {t('departments.title')}
          </h1>
          <p className="text-muted-foreground text-sm mt-1">
            {t('departments.subtitle')}
          </p>
        </div>
        <Button onClick={() => { setForm({ ...emptyForm }); setCreateOpen(true); }}>
          <Plus className="size-4 mr-1" />
          {t('departments.addDept')}
        </Button>
      </div>

      {/* Department cards */}
      {departments.length === 0 ? (
        <Card>
          <CardContent className="py-12 text-center text-muted-foreground">
            {t('departments.noDepts')}
          </CardContent>
        </Card>
      ) : (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {departments.map((dept) => (
            <Card key={dept.id}>
              <CardHeader className="pb-2">
                <div className="flex items-start justify-between">
                  <div>
                    <CardTitle className="text-base">{dept.name}</CardTitle>
                    <CardDescription className="text-xs font-mono">{dept.id}</CardDescription>
                  </div>
                  <div className="flex gap-1">
                    <Button variant="ghost" size="icon-xs" onClick={() => openEdit(dept)}>
                      <Pencil className="size-3.5" />
                    </Button>
                    <Button variant="ghost" size="icon-xs" onClick={() => openDelete(dept.id)}>
                      <Trash2 className="size-3.5 text-destructive" />
                    </Button>
                  </div>
                </div>
              </CardHeader>
              <CardContent className="space-y-2 text-sm">
                {dept.description && (
                  <p className="text-muted-foreground">{dept.description}</p>
                )}
                {dept.parent_id && (
                  <div className="flex items-center gap-1">
                    <span className="text-muted-foreground">{t('departments.parent')}:</span>
                    <Badge variant="secondary" className="text-xs">
                      {parentName(dept.parent_id)}
                    </Badge>
                  </div>
                )}
                <div className="flex items-center gap-1 text-muted-foreground">
                  <Users className="size-3.5" />
                  <span>{t('departments.agentCount')}: {dept.agent_count}</span>
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      )}

      {/* Create Dialog */}
      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('departments.addDept')}</DialogTitle>
            <DialogDescription>{t('departments.subtitle')}</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleCreate} className="space-y-4">
            <div className="space-y-2">
              <Label>{t('departments.name')}</Label>
              <Input
                required
                value={form.name}
                onChange={(e) => setForm({ ...form, name: e.target.value })}
              />
            </div>
            <div className="space-y-2">
              <Label>{t('departments.description')}</Label>
              <Input
                value={form.description}
                onChange={(e) => setForm({ ...form, description: e.target.value })}
              />
            </div>
            <div className="space-y-2">
              <Label>{t('departments.parent')}</Label>
              <select
                className={selectClasses}
                value={form.parent_id}
                onChange={(e) => setForm({ ...form, parent_id: e.target.value })}
              >
                <option value="">{t('departments.noParent')}</option>
                {departments.map((d) => (
                  <option key={d.id} value={d.id}>
                    {d.name}
                  </option>
                ))}
              </select>
            </div>
            <DialogFooter>
              <Button variant="outline" type="button" onClick={() => setCreateOpen(false)}>
                {t('common.cancel')}
              </Button>
              <Button type="submit">{t('common.save')}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* Edit Dialog */}
      <Dialog open={editOpen} onOpenChange={setEditOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('departments.editDept')}</DialogTitle>
            <DialogDescription>{editId}</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleEdit} className="space-y-4">
            <div className="space-y-2">
              <Label>{t('departments.name')}</Label>
              <Input
                required
                value={form.name}
                onChange={(e) => setForm({ ...form, name: e.target.value })}
              />
            </div>
            <div className="space-y-2">
              <Label>{t('departments.description')}</Label>
              <Input
                value={form.description}
                onChange={(e) => setForm({ ...form, description: e.target.value })}
              />
            </div>
            <div className="space-y-2">
              <Label>{t('departments.parent')}</Label>
              <select
                className={selectClasses}
                value={form.parent_id}
                onChange={(e) => setForm({ ...form, parent_id: e.target.value })}
              >
                <option value="">{t('departments.noParent')}</option>
                {departments
                  .filter((d) => d.id !== editId)
                  .map((d) => (
                    <option key={d.id} value={d.id}>
                      {d.name}
                    </option>
                  ))}
              </select>
            </div>
            <DialogFooter>
              <Button variant="outline" type="button" onClick={() => setEditOpen(false)}>
                {t('common.cancel')}
              </Button>
              <Button type="submit">{t('common.save')}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* Delete Confirmation */}
      <Dialog open={deleteOpen} onOpenChange={setDeleteOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('departments.deleteDept')}</DialogTitle>
            <DialogDescription>{t('departments.deleteConfirm')}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteOpen(false)}>
              {t('common.cancel')}
            </Button>
            <Button variant="destructive" onClick={() => deleteMut.mutate(deleteId)}>
              {t('common.delete')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
