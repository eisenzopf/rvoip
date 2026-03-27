import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Badge } from '@/components/ui/badge';
import { Checkbox } from '@/components/ui/checkbox';
import { Switch } from '@/components/ui/switch';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog';
import { Plus, Pencil, Trash2, ShieldCheck, Search } from 'lucide-react';
import {
  fetchUsers,
  createUser,
  updateUser,
  deleteUser,
  updateUserRoles,
} from '@/lib/api';
import type { UserView, CreateUserRequest, UpdateUserRequest } from '@/lib/api';
import { useAuth } from '@/hooks/useAuth';

const AVAILABLE_ROLES = ['super_admin', 'admin', 'supervisor', 'agent'];

export function Users() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const { hasRole } = useAuth();
  const isSuperAdmin = hasRole('super_admin');

  const [search, setSearch] = useState('');
  const [createOpen, setCreateOpen] = useState(false);
  const [editOpen, setEditOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [rolesOpen, setRolesOpen] = useState(false);
  const [selectedUser, setSelectedUser] = useState<UserView | null>(null);

  // Create form state
  const [createForm, setCreateForm] = useState<CreateUserRequest>({
    username: '',
    password: '',
    email: '',
    display_name: '',
    roles: [],
  });

  // Edit form state
  const [editForm, setEditForm] = useState<UpdateUserRequest>({
    email: '',
    display_name: '',
    active: true,
  });

  // Roles form state
  const [rolesForm, setRolesForm] = useState<string[]>([]);

  const { data } = useQuery({
    queryKey: ['users', search],
    queryFn: () => fetchUsers({ search: search || undefined }),
    refetchInterval: 10000,
  });

  const users: UserView[] = data?.users ?? [];
  const total = data?.total ?? 0;

  const createMutation = useMutation({
    mutationFn: (data: CreateUserRequest) => createUser(data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['users'] });
      setCreateOpen(false);
      setCreateForm({ username: '', password: '', email: '', display_name: '', roles: [] });
    },
  });

  const updateMutation = useMutation({
    mutationFn: (vars: { id: string; data: UpdateUserRequest }) => updateUser(vars.id, vars.data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['users'] });
      setEditOpen(false);
      setSelectedUser(null);
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => deleteUser(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['users'] });
      setDeleteOpen(false);
      setSelectedUser(null);
    },
  });

  const rolesMutation = useMutation({
    mutationFn: (vars: { id: string; roles: string[] }) => updateUserRoles(vars.id, vars.roles),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['users'] });
      setRolesOpen(false);
      setSelectedUser(null);
    },
  });

  function handleCreateSubmit(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    createMutation.mutate({
      ...createForm,
      email: createForm.email || undefined,
      display_name: createForm.display_name || undefined,
    });
  }

  function handleEditSubmit(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    if (!selectedUser) return;
    updateMutation.mutate({
      id: selectedUser.id,
      data: {
        email: editForm.email || undefined,
        display_name: editForm.display_name || undefined,
        active: editForm.active,
      },
    });
  }

  function handleRolesSubmit(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    if (!selectedUser) return;
    rolesMutation.mutate({ id: selectedUser.id, roles: rolesForm });
  }

  function openEdit(user: UserView) {
    setSelectedUser(user);
    setEditForm({
      email: user.email ?? '',
      display_name: user.display_name ?? '',
      active: user.active,
    });
    setEditOpen(true);
  }

  function openDelete(user: UserView) {
    setSelectedUser(user);
    setDeleteOpen(true);
  }

  function openRoles(user: UserView) {
    setSelectedUser(user);
    setRolesForm([...user.roles]);
    setRolesOpen(true);
  }

  function toggleCreateRole(role: string) {
    setCreateForm((prev) => ({
      ...prev,
      roles: prev.roles.includes(role)
        ? prev.roles.filter((r) => r !== role)
        : [...prev.roles, role],
    }));
  }

  function toggleRolesFormRole(role: string) {
    setRolesForm((prev) =>
      prev.includes(role) ? prev.filter((r) => r !== role) : [...prev, role],
    );
  }

  return (
    <div className="p-6 space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">{t('users.title')}</h1>
          <p className="text-sm text-muted-foreground">{t('users.subtitle')}</p>
        </div>
        <Dialog open={createOpen} onOpenChange={setCreateOpen}>
          <DialogTrigger render={<Button size="sm" />}>
            <Plus className="size-4 mr-1.5" />
            {t('users.addUser')}
          </DialogTrigger>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>{t('users.addUser')}</DialogTitle>
              <DialogDescription>{t('users.subtitle')}</DialogDescription>
            </DialogHeader>
            <form onSubmit={handleCreateSubmit} className="space-y-4">
              <div className="space-y-2">
                <Label htmlFor="create-username">{t('users.username')}</Label>
                <Input
                  id="create-username"
                  value={createForm.username}
                  onChange={(e) => setCreateForm({ ...createForm, username: e.target.value })}
                  required
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="create-password">{t('users.password')}</Label>
                <Input
                  id="create-password"
                  type="password"
                  value={createForm.password}
                  onChange={(e) => setCreateForm({ ...createForm, password: e.target.value })}
                  required
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="create-email">{t('users.email')}</Label>
                <Input
                  id="create-email"
                  type="email"
                  value={createForm.email ?? ''}
                  onChange={(e) => setCreateForm({ ...createForm, email: e.target.value })}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="create-display-name">{t('users.displayName')}</Label>
                <Input
                  id="create-display-name"
                  value={createForm.display_name ?? ''}
                  onChange={(e) => setCreateForm({ ...createForm, display_name: e.target.value })}
                />
              </div>
              <div className="space-y-2">
                <Label>{t('users.roles')}</Label>
                <div className="flex flex-wrap gap-3">
                  {AVAILABLE_ROLES.map((role) => (
                    <label key={role} className="flex items-center gap-2 text-sm cursor-pointer">
                      <Checkbox
                        checked={createForm.roles.includes(role)}
                        onCheckedChange={() => toggleCreateRole(role)}
                      />
                      {role}
                    </label>
                  ))}
                </div>
              </div>
              <DialogFooter>
                <Button type="button" variant="outline" onClick={() => setCreateOpen(false)}>
                  {t('common.cancel')}
                </Button>
                <Button type="submit" disabled={createMutation.isPending}>
                  {t('users.form.create')}
                </Button>
              </DialogFooter>
            </form>
          </DialogContent>
        </Dialog>
      </div>

      {/* Search */}
      <div className="relative max-w-sm">
        <Search className="absolute left-3 top-1/2 -translate-y-1/2 size-4 text-muted-foreground" />
        <Input
          className="pl-9"
          placeholder={t('users.searchPlaceholder')}
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
      </div>

      {/* Count */}
      <p className="text-sm text-muted-foreground">{t('users.totalUsers', { count: total })}</p>

      {/* Table */}
      {users.length === 0 ? (
        <div className="py-16 text-center text-muted-foreground text-sm">
          {t('users.noUsers')}
        </div>
      ) : (
        <div className="rounded-md border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t('users.username')}</TableHead>
                <TableHead>{t('users.displayName')}</TableHead>
                <TableHead>{t('users.email')}</TableHead>
                <TableHead>{t('users.roles')}</TableHead>
                <TableHead>{t('users.active')}</TableHead>
                <TableHead>{t('users.createdAt')}</TableHead>
                <TableHead>{t('users.lastLogin')}</TableHead>
                <TableHead className="text-right" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {users.map((user) => (
                <TableRow key={user.id}>
                  <TableCell className="font-medium">{user.username}</TableCell>
                  <TableCell>{user.display_name ?? '-'}</TableCell>
                  <TableCell className="text-muted-foreground">{user.email ?? '-'}</TableCell>
                  <TableCell>
                    <div className="flex flex-wrap gap-1">
                      {user.roles.map((role) => (
                        <Badge key={role} variant="secondary" className="text-[10px]">
                          {role}
                        </Badge>
                      ))}
                    </div>
                  </TableCell>
                  <TableCell>
                    {user.active ? (
                      <span className="inline-flex items-center gap-1.5 text-xs">
                        <span className="h-2 w-2 rounded-full bg-green-500" />
                        {t('users.activeYes')}
                      </span>
                    ) : (
                      <span className="inline-flex items-center gap-1.5 text-xs">
                        <span className="h-2 w-2 rounded-full bg-red-500" />
                        {t('users.activeNo')}
                      </span>
                    )}
                  </TableCell>
                  <TableCell className="text-xs text-muted-foreground">
                    {new Date(user.created_at).toLocaleDateString()}
                  </TableCell>
                  <TableCell className="text-xs text-muted-foreground">
                    {user.last_login ? new Date(user.last_login).toLocaleString() : '-'}
                  </TableCell>
                  <TableCell className="text-right">
                    <div className="flex items-center justify-end gap-1">
                      {isSuperAdmin && (
                        <Button
                          variant="ghost"
                          size="sm"
                          className="h-7 w-7"
                          onClick={() => openRoles(user)}
                          title={t('users.assignRoles')}
                        >
                          <ShieldCheck className="size-3.5" />
                        </Button>
                      )}
                      <Button
                        variant="ghost"
                        size="sm"
                        className="h-7 w-7"
                        onClick={() => openEdit(user)}
                        title={t('users.editUser')}
                      >
                        <Pencil className="size-3.5" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        className="h-7 w-7 text-destructive hover:text-destructive"
                        onClick={() => openDelete(user)}
                        title={t('users.deleteUser')}
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

      {/* Edit Dialog */}
      <Dialog open={editOpen} onOpenChange={setEditOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('users.editUser')}</DialogTitle>
            <DialogDescription>{selectedUser?.username}</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleEditSubmit} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="edit-email">{t('users.email')}</Label>
              <Input
                id="edit-email"
                type="email"
                value={editForm.email ?? ''}
                onChange={(e) => setEditForm({ ...editForm, email: e.target.value })}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="edit-display-name">{t('users.displayName')}</Label>
              <Input
                id="edit-display-name"
                value={editForm.display_name ?? ''}
                onChange={(e) => setEditForm({ ...editForm, display_name: e.target.value })}
              />
            </div>
            <div className="flex items-center gap-3">
              <Label htmlFor="edit-active">{t('users.active')}</Label>
              <Switch
                id="edit-active"
                checked={editForm.active ?? true}
                onCheckedChange={(checked: boolean) => setEditForm({ ...editForm, active: checked })}
              />
              <span className="text-sm text-muted-foreground">
                {editForm.active ? t('users.activeYes') : t('users.activeNo')}
              </span>
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setEditOpen(false)}>
                {t('common.cancel')}
              </Button>
              <Button type="submit" disabled={updateMutation.isPending}>
                {t('users.form.update')}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* Delete Dialog */}
      <Dialog open={deleteOpen} onOpenChange={setDeleteOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('users.deleteUser')}</DialogTitle>
            <DialogDescription>{t('users.deleteConfirm')}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteOpen(false)}>
              {t('common.cancel')}
            </Button>
            <Button
              variant="destructive"
              onClick={() => { if (selectedUser) deleteMutation.mutate(selectedUser.id); }}
              disabled={deleteMutation.isPending}
            >
              {t('common.delete')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Assign Roles Dialog */}
      <Dialog open={rolesOpen} onOpenChange={setRolesOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('users.assignRoles')}</DialogTitle>
            <DialogDescription>{selectedUser?.username}</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleRolesSubmit} className="space-y-4">
            <div className="flex flex-col gap-3">
              {AVAILABLE_ROLES.map((role) => (
                <label key={role} className="flex items-center gap-2 text-sm cursor-pointer">
                  <Checkbox
                    checked={rolesForm.includes(role)}
                    onCheckedChange={() => toggleRolesFormRole(role)}
                  />
                  {role}
                </label>
              ))}
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setRolesOpen(false)}>
                {t('common.cancel')}
              </Button>
              <Button type="submit" disabled={rolesMutation.isPending}>
                {t('common.save')}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
    </div>
  );
}
