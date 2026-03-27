import { useState, useMemo } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { Card, CardContent } from '@/components/ui/card';
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
import { Plus, Pencil, Trash2, Search, ShieldBan, Star } from 'lucide-react';
import {
  fetchPhoneLists,
  createPhoneList,
  updatePhoneList,
  deletePhoneList,
} from '@/lib/api';
import type { PhoneListEntry, CreatePhoneListRequest, UpdatePhoneListRequest } from '@/lib/api';

type ListType = 'blacklist' | 'whitelist' | 'vip';

interface FormState {
  number: string;
  list_type: ListType;
  reason: string;
  customer_name: string;
  vip_level: number;
  expires_at: string;
  created_by: string;
}

const defaultForm: FormState = {
  number: '',
  list_type: 'blacklist',
  reason: '',
  customer_name: '',
  vip_level: 3,
  expires_at: '',
  created_by: '',
};

function VipStars({ level }: { level: number }) {
  return (
    <span className="flex items-center gap-0.5">
      {Array.from({ length: 5 }, (_, i) => (
        <Star
          key={i}
          className={`size-3.5 ${i < level ? 'fill-yellow-400 text-yellow-400' : 'text-muted-foreground/30'}`}
        />
      ))}
    </span>
  );
}

export function PhoneLists() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();

  const [activeTab, setActiveTab] = useState<ListType>('blacklist');
  const [search, setSearch] = useState('');
  const [createOpen, setCreateOpen] = useState(false);
  const [editOpen, setEditOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [editId, setEditId] = useState('');
  const [deleteId, setDeleteId] = useState('');
  const [form, setForm] = useState<FormState>({ ...defaultForm });

  const { data: allEntries } = useQuery<PhoneListEntry[]>({
    queryKey: ['phone-lists'],
    queryFn: () => fetchPhoneLists(),
  });

  const entries = useMemo(() => {
    if (!allEntries) return [];
    return allEntries.filter((e) => {
      if (e.list_type !== activeTab) return false;
      if (search) {
        const q = search.toLowerCase();
        return (
          e.number.toLowerCase().includes(q) ||
          (e.customer_name ?? '').toLowerCase().includes(q) ||
          (e.reason ?? '').toLowerCase().includes(q)
        );
      }
      return true;
    });
  }, [allEntries, activeTab, search]);

  const tabCounts = useMemo(() => {
    if (!allEntries) return { blacklist: 0, whitelist: 0, vip: 0 };
    return {
      blacklist: allEntries.filter((e) => e.list_type === 'blacklist').length,
      whitelist: allEntries.filter((e) => e.list_type === 'whitelist').length,
      vip: allEntries.filter((e) => e.list_type === 'vip').length,
    };
  }, [allEntries]);

  const createMut = useMutation({
    mutationFn: (data: CreatePhoneListRequest) => createPhoneList(data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['phone-lists'] });
      setCreateOpen(false);
      setForm({ ...defaultForm });
    },
  });

  const updateMut = useMutation({
    mutationFn: ({ id, data }: { id: string; data: UpdatePhoneListRequest }) =>
      updatePhoneList(id, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['phone-lists'] });
      setEditOpen(false);
      setForm({ ...defaultForm });
    },
  });

  const deleteMut = useMutation({
    mutationFn: (id: string) => deletePhoneList(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['phone-lists'] });
      setDeleteOpen(false);
    },
  });

  const handleCreate = (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    if (!form.number.trim()) return;
    createMut.mutate({
      number: form.number,
      list_type: form.list_type,
      reason: form.reason || undefined,
      customer_name: form.customer_name || undefined,
      vip_level: form.list_type === 'vip' ? form.vip_level : undefined,
      expires_at: form.expires_at || undefined,
      created_by: form.created_by || undefined,
    });
  };

  const handleUpdate = (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    updateMut.mutate({
      id: editId,
      data: {
        number: form.number || undefined,
        list_type: form.list_type,
        reason: form.reason || undefined,
        customer_name: form.customer_name || undefined,
        vip_level: form.list_type === 'vip' ? form.vip_level : undefined,
        expires_at: form.expires_at || undefined,
        created_by: form.created_by || undefined,
      },
    });
  };

  const openEdit = (entry: PhoneListEntry) => {
    setEditId(entry.id);
    setForm({
      number: entry.number,
      list_type: entry.list_type as ListType,
      reason: entry.reason ?? '',
      customer_name: entry.customer_name ?? '',
      vip_level: entry.vip_level ?? 3,
      expires_at: entry.expires_at ?? '',
      created_by: entry.created_by ?? '',
    });
    setEditOpen(true);
  };

  const openCreate = () => {
    setForm({ ...defaultForm, list_type: activeTab });
    setCreateOpen(true);
  };

  const tabBadgeVariant = (tab: ListType) =>
    activeTab === tab ? 'default' as const : 'secondary' as const;

  const typeLabel = (lt: string) => {
    if (lt === 'blacklist') return t('phoneLists.blacklist');
    if (lt === 'whitelist') return t('phoneLists.whitelist');
    return t('phoneLists.vip');
  };

  const typeBadgeVariant = (lt: string) => {
    if (lt === 'blacklist') return 'destructive' as const;
    if (lt === 'vip') return 'default' as const;
    return 'secondary' as const;
  };

  const renderFormFields = () => (
    <>
      <div>
        <Label>{t('phoneLists.number')}</Label>
        <Input
          value={form.number}
          onChange={(e) => setForm({ ...form, number: e.target.value })}
          placeholder="+86..."
          required
        />
      </div>
      <div>
        <Label>{t('phoneLists.type')}</Label>
        <select
          className="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          value={form.list_type}
          onChange={(e) => setForm({ ...form, list_type: e.target.value as ListType })}
        >
          <option value="blacklist">{t('phoneLists.blacklist')}</option>
          <option value="whitelist">{t('phoneLists.whitelist')}</option>
          <option value="vip">{t('phoneLists.vip')}</option>
        </select>
      </div>
      <div>
        <Label>{t('phoneLists.reason')}</Label>
        <Input
          value={form.reason}
          onChange={(e) => setForm({ ...form, reason: e.target.value })}
        />
      </div>
      {(form.list_type === 'whitelist' || form.list_type === 'vip') && (
        <div>
          <Label>{t('phoneLists.customerName')}</Label>
          <Input
            value={form.customer_name}
            onChange={(e) => setForm({ ...form, customer_name: e.target.value })}
          />
        </div>
      )}
      {form.list_type === 'vip' && (
        <div>
          <Label>{t('phoneLists.vipLevel')} ({form.vip_level})</Label>
          <div className="flex items-center gap-1 mt-1">
            {[1, 2, 3, 4, 5].map((lvl) => (
              <button
                key={lvl}
                type="button"
                onClick={() => setForm({ ...form, vip_level: lvl })}
                className="p-0.5"
              >
                <Star
                  className={`size-5 ${lvl <= form.vip_level ? 'fill-yellow-400 text-yellow-400' : 'text-muted-foreground/30'}`}
                />
              </button>
            ))}
          </div>
        </div>
      )}
      <div>
        <Label>{t('phoneLists.expiresAt')}</Label>
        <Input
          type="datetime-local"
          value={form.expires_at ? form.expires_at.slice(0, 16) : ''}
          onChange={(e) => setForm({ ...form, expires_at: e.target.value ? new Date(e.target.value).toISOString() : '' })}
        />
      </div>
      <div>
        <Label>{t('phoneLists.createdBy')}</Label>
        <Input
          value={form.created_by}
          onChange={(e) => setForm({ ...form, created_by: e.target.value })}
        />
      </div>
    </>
  );

  return (
    <div className="p-6 space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold flex items-center gap-2">
            <ShieldBan className="size-6" />
            {t('phoneLists.title')}
          </h1>
          <p className="text-muted-foreground text-sm">{t('phoneLists.subtitle')}</p>
        </div>
        <Dialog open={createOpen} onOpenChange={setCreateOpen}>
          <DialogTrigger render={<Button size="sm" onClick={openCreate} />}>
            <Plus className="size-4 mr-1" />{t('phoneLists.add')}
          </DialogTrigger>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>{t('phoneLists.add')}</DialogTitle>
              <DialogDescription>{t('phoneLists.subtitle')}</DialogDescription>
            </DialogHeader>
            <form onSubmit={handleCreate} className="space-y-4">
              {renderFormFields()}
              <DialogFooter>
                <Button type="button" variant="outline" onClick={() => setCreateOpen(false)}>{t('common.cancel')}</Button>
                <Button type="submit">{t('common.save')}</Button>
              </DialogFooter>
            </form>
          </DialogContent>
        </Dialog>
      </div>

      {/* Tabs */}
      <div className="flex items-center gap-2">
        {(['blacklist', 'whitelist', 'vip'] as ListType[]).map((tab) => (
          <Button
            key={tab}
            variant={activeTab === tab ? 'default' : 'outline'}
            size="sm"
            onClick={() => setActiveTab(tab)}
            className="gap-1.5"
          >
            {typeLabel(tab)}
            <Badge variant={tabBadgeVariant(tab)} className="text-xs ml-1">
              {tabCounts[tab]}
            </Badge>
          </Button>
        ))}
        <div className="ml-auto relative">
          <Search className="absolute left-2.5 top-2.5 size-4 text-muted-foreground" />
          <Input
            className="pl-8 w-64"
            placeholder={t('phoneLists.searchPlaceholder')}
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
        </div>
      </div>

      {/* Table */}
      {entries.length === 0 ? (
        <Card>
          <CardContent className="py-12 text-center text-muted-foreground">
            {t('phoneLists.noEntries')}
          </CardContent>
        </Card>
      ) : (
        <Card>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t('phoneLists.number')}</TableHead>
                <TableHead>{t('phoneLists.type')}</TableHead>
                {activeTab === 'vip' && <TableHead>{t('phoneLists.vipLevel')}</TableHead>}
                <TableHead>{activeTab === 'vip' ? t('phoneLists.customerName') : t('phoneLists.reason')}</TableHead>
                {(activeTab === 'whitelist') && <TableHead>{t('phoneLists.customerName')}</TableHead>}
                <TableHead>{t('phoneLists.expiresAt')}</TableHead>
                <TableHead>{t('phoneLists.createdBy')}</TableHead>
                <TableHead className="w-24" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {entries.map((entry) => (
                <TableRow key={entry.id}>
                  <TableCell className="font-mono">{entry.number}</TableCell>
                  <TableCell>
                    <Badge variant={typeBadgeVariant(entry.list_type)}>
                      {typeLabel(entry.list_type)}
                    </Badge>
                  </TableCell>
                  {activeTab === 'vip' && (
                    <TableCell>
                      {entry.vip_level != null ? <VipStars level={entry.vip_level} /> : '-'}
                    </TableCell>
                  )}
                  <TableCell>
                    {activeTab === 'vip'
                      ? (entry.customer_name ?? '-')
                      : (entry.reason ?? '-')}
                  </TableCell>
                  {activeTab === 'whitelist' && (
                    <TableCell>{entry.customer_name ?? '-'}</TableCell>
                  )}
                  <TableCell className="text-sm text-muted-foreground">
                    {entry.expires_at
                      ? new Date(entry.expires_at).toLocaleString()
                      : '-'}
                  </TableCell>
                  <TableCell className="text-sm">{entry.created_by ?? '-'}</TableCell>
                  <TableCell>
                    <div className="flex gap-1">
                      <Button variant="ghost" size="icon-xs" onClick={() => openEdit(entry)}>
                        <Pencil className="size-3.5" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon-xs"
                        onClick={() => { setDeleteId(entry.id); setDeleteOpen(true); }}
                      >
                        <Trash2 className="size-3.5 text-destructive" />
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </Card>
      )}

      {/* Edit dialog */}
      <Dialog open={editOpen} onOpenChange={setEditOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('phoneLists.edit')}</DialogTitle>
            <DialogDescription>{editId}</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleUpdate} className="space-y-4">
            {renderFormFields()}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setEditOpen(false)}>{t('common.cancel')}</Button>
              <Button type="submit">{t('common.save')}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* Delete confirmation */}
      <Dialog open={deleteOpen} onOpenChange={setDeleteOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('common.confirm')}</DialogTitle>
            <DialogDescription>{t('phoneLists.deleteConfirm')}</DialogDescription>
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
