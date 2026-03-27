import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { Card, CardContent, CardHeader, CardTitle, CardAction } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Textarea } from '@/components/ui/textarea';
import { Switch } from '@/components/ui/switch';
import { Checkbox } from '@/components/ui/checkbox';
import {
  Dialog, DialogContent, DialogDescription, DialogFooter,
  DialogHeader, DialogTitle,
} from '@/components/ui/dialog';
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from '@/components/ui/select';
import { Plus, Pencil, Trash2, ChevronDown, ChevronRight, Star, Clock } from 'lucide-react';
import {
  fetchIvrMenus, createIvrMenu, updateIvrMenu, deleteIvrMenu,
  createIvrOption, updateIvrOption, deleteIvrOption,
} from '@/lib/api';
import type {
  IvrMenuView, IvrOptionView,
  CreateIvrMenuRequest, UpdateIvrMenuRequest,
  CreateIvrOptionRequest, UpdateIvrOptionRequest,
} from '@/lib/api';

const DIGITS = ['0','1','2','3','4','5','6','7','8','9','*','#'];
const ACTION_TYPES = ['route_to_queue','route_to_agent','sub_menu','transfer','voicemail','hangup'];
const TIMEOUT_ACTIONS = ['repeat','voicemail','hangup'];
const DAYS = [
  { key: '1', label: 'Mon' }, { key: '2', label: 'Tue' }, { key: '3', label: 'Wed' },
  { key: '4', label: 'Thu' }, { key: '5', label: 'Fri' }, { key: '6', label: 'Sat' },
  { key: '7', label: 'Sun' },
];

interface MenuForm {
  name: string; description: string; welcome_message: string;
  timeout_seconds: number; max_retries: number;
  timeout_action: string; invalid_action: string;
  is_root: boolean; business_hours_start: string; business_hours_end: string;
  business_days: string; after_hours_action: string;
}

interface OptionForm {
  digit: string; label: string; action_type: string;
  action_target: string; announcement: string;
}

const defaultMenuForm: MenuForm = {
  name: '', description: '', welcome_message: '',
  timeout_seconds: 10, max_retries: 3,
  timeout_action: 'repeat', invalid_action: 'repeat',
  is_root: false, business_hours_start: '09:00', business_hours_end: '18:00',
  business_days: '1,2,3,4,5', after_hours_action: 'voicemail',
};

const defaultOptionForm: OptionForm = {
  digit: '0', label: '', action_type: 'route_to_queue',
  action_target: '', announcement: '',
};

function actionLabel(t: (k: string) => string, action: string): string {
  const map: Record<string, string> = {
    route_to_queue: t('ivr.routeToQueue'), route_to_agent: t('ivr.routeToAgent'),
    sub_menu: t('ivr.subMenu'), transfer: t('ivr.transfer'),
    voicemail: t('ivr.voicemail'), hangup: t('ivr.hangup'), repeat: t('ivr.repeat'),
  };
  return map[action] ?? action;
}

export function Ivr() {
  const { t } = useTranslation();
  const qc = useQueryClient();
  const [expanded, setExpanded] = useState<string | null>(null);
  const [menuDialogOpen, setMenuDialogOpen] = useState(false);
  const [editingMenu, setEditingMenu] = useState<IvrMenuView | null>(null);
  const [menuForm, setMenuForm] = useState<MenuForm>(defaultMenuForm);
  const [optDialogOpen, setOptDialogOpen] = useState(false);
  const [editingOpt, setEditingOpt] = useState<IvrOptionView | null>(null);
  const [optMenuId, setOptMenuId] = useState('');
  const [optForm, setOptForm] = useState<OptionForm>(defaultOptionForm);

  const { data: menus = [] } = useQuery({ queryKey: ['ivr'], queryFn: fetchIvrMenus });

  const invalidate = () => qc.invalidateQueries({ queryKey: ['ivr'] });

  const createMenuMut = useMutation({ mutationFn: (d: CreateIvrMenuRequest) => createIvrMenu(d), onSuccess: invalidate });
  const updateMenuMut = useMutation({ mutationFn: ({ id, d }: { id: string; d: UpdateIvrMenuRequest }) => updateIvrMenu(id, d), onSuccess: invalidate });
  const deleteMenuMut = useMutation({ mutationFn: (id: string) => deleteIvrMenu(id), onSuccess: invalidate });
  const createOptMut = useMutation({ mutationFn: ({ mid, d }: { mid: string; d: CreateIvrOptionRequest }) => createIvrOption(mid, d), onSuccess: invalidate });
  const updateOptMut = useMutation({ mutationFn: ({ mid, oid, d }: { mid: string; oid: string; d: UpdateIvrOptionRequest }) => updateIvrOption(mid, oid, d), onSuccess: invalidate });
  const deleteOptMut = useMutation({ mutationFn: ({ mid, oid }: { mid: string; oid: string }) => deleteIvrOption(mid, oid), onSuccess: invalidate });

  const openCreateMenu = () => {
    setEditingMenu(null);
    setMenuForm(defaultMenuForm);
    setMenuDialogOpen(true);
  };

  const openEditMenu = (m: IvrMenuView) => {
    setEditingMenu(m);
    setMenuForm({
      name: m.name, description: m.description ?? '', welcome_message: m.welcome_message ?? '',
      timeout_seconds: m.timeout_seconds, max_retries: m.max_retries,
      timeout_action: m.timeout_action, invalid_action: m.invalid_action,
      is_root: m.is_root, business_hours_start: m.business_hours_start,
      business_hours_end: m.business_hours_end, business_days: m.business_days,
      after_hours_action: m.after_hours_action,
    });
    setMenuDialogOpen(true);
  };

  const submitMenu = (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    if (!menuForm.name.trim()) return;
    const payload = { ...menuForm, description: menuForm.description || undefined, welcome_message: menuForm.welcome_message || undefined };
    if (editingMenu) {
      updateMenuMut.mutate({ id: editingMenu.id, d: payload });
    } else {
      createMenuMut.mutate(payload);
    }
    setMenuDialogOpen(false);
  };

  const openCreateOpt = (menuId: string) => {
    setEditingOpt(null);
    setOptMenuId(menuId);
    setOptForm(defaultOptionForm);
    setOptDialogOpen(true);
  };

  const openEditOpt = (menuId: string, opt: IvrOptionView) => {
    setEditingOpt(opt);
    setOptMenuId(menuId);
    setOptForm({
      digit: opt.digit, label: opt.label, action_type: opt.action_type,
      action_target: opt.action_target ?? '', announcement: opt.announcement ?? '',
    });
    setOptDialogOpen(true);
  };

  const submitOpt = (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    if (!optForm.label.trim()) return;
    const payload = { ...optForm, action_target: optForm.action_target || undefined, announcement: optForm.announcement || undefined };
    if (editingOpt) {
      updateOptMut.mutate({ mid: optMenuId, oid: editingOpt.id, d: payload });
    } else {
      createOptMut.mutate({ mid: optMenuId, d: payload });
    }
    setOptDialogOpen(false);
  };

  const toggleDay = (dayKey: string) => {
    const days = menuForm.business_days ? menuForm.business_days.split(',') : [];
    const next = days.includes(dayKey) ? days.filter(d => d !== dayKey) : [...days, dayKey];
    setMenuForm({ ...menuForm, business_days: next.sort().join(',') });
  };

  const selectedDays = menuForm.business_days ? menuForm.business_days.split(',') : [];

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">{t('ivr.title')}</h1>
          <p className="text-muted-foreground">{t('ivr.subtitle')}</p>
        </div>
        <Button onClick={openCreateMenu}><Plus className="mr-2 h-4 w-4" />{t('ivr.addMenu')}</Button>
      </div>

      {/* Menu list */}
      {menus.length === 0 && (
        <Card><CardContent className="py-8 text-center text-muted-foreground">{t('ivr.noMenus')}</CardContent></Card>
      )}

      {menus.map(menu => {
        const isExpanded = expanded === menu.id;
        return (
          <Card key={menu.id}>
            <CardHeader className="cursor-pointer" onClick={() => setExpanded(isExpanded ? null : menu.id)}>
              <CardTitle className="flex items-center gap-2">
                {isExpanded ? <ChevronDown className="h-4 w-4" /> : <ChevronRight className="h-4 w-4" />}
                {menu.name}
                {menu.is_root && <Badge variant="outline" className="gap-1"><Star className="h-3 w-3" />Root</Badge>}
                <Badge variant="secondary" className="gap-1"><Clock className="h-3 w-3" />{menu.business_hours_start}-{menu.business_hours_end}</Badge>
                <Badge variant="secondary">{menu.options.length} {t('ivr.options')}</Badge>
              </CardTitle>
              <CardAction>
                <div className="flex gap-1">
                  <Button variant="ghost" size="icon" onClick={(e) => { e.stopPropagation(); openEditMenu(menu); }}>
                    <Pencil className="h-4 w-4" />
                  </Button>
                  <Button variant="ghost" size="icon" onClick={(e) => { e.stopPropagation(); deleteMenuMut.mutate(menu.id); }}>
                    <Trash2 className="h-4 w-4" />
                  </Button>
                </div>
              </CardAction>
            </CardHeader>

            {isExpanded && (
              <CardContent className="space-y-3">
                {menu.options.sort((a, b) => a.position - b.position).map(opt => (
                  <div key={opt.id} className="flex items-center gap-3 rounded-md border p-2">
                    <Badge className="w-8 justify-center">{opt.digit}</Badge>
                    <span className="flex-1 text-sm">{opt.label}</span>
                    <Badge variant="outline">{actionLabel(t, opt.action_type)}</Badge>
                    {opt.action_target && <span className="text-xs text-muted-foreground">{opt.action_target}</span>}
                    <Button variant="ghost" size="icon" onClick={() => openEditOpt(menu.id, opt)}><Pencil className="h-3 w-3" /></Button>
                    <Button variant="ghost" size="icon" onClick={() => deleteOptMut.mutate({ mid: menu.id, oid: opt.id })}><Trash2 className="h-3 w-3" /></Button>
                  </div>
                ))}
                <Button variant="outline" size="sm" onClick={() => openCreateOpt(menu.id)}>
                  <Plus className="mr-1 h-3 w-3" />{t('ivr.addOption')}
                </Button>
              </CardContent>
            )}
          </Card>
        );
      })}

      {/* Menu Dialog */}
      <Dialog open={menuDialogOpen} onOpenChange={setMenuDialogOpen}>
        <DialogContent className="max-w-lg max-h-[85vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>{editingMenu ? t('ivr.editMenu') : t('ivr.addMenu')}</DialogTitle>
            <DialogDescription>{editingMenu ? t('ivr.editMenu') : t('ivr.addMenu')}</DialogDescription>
          </DialogHeader>
          <form onSubmit={submitMenu} className="space-y-4">
            <div className="space-y-2">
              <Label>{t('ivr.menuName')}</Label>
              <Input value={menuForm.name} onChange={e => setMenuForm({ ...menuForm, name: e.target.value })} required />
            </div>
            <div className="space-y-2">
              <Label>{t('ivr.welcomeMsg')}</Label>
              <Textarea value={menuForm.welcome_message} onChange={e => setMenuForm({ ...menuForm, welcome_message: e.target.value })} />
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label>{t('ivr.timeout')}</Label>
                <Input type="number" value={menuForm.timeout_seconds} onChange={e => setMenuForm({ ...menuForm, timeout_seconds: Number(e.target.value) })} />
              </div>
              <div className="space-y-2">
                <Label>{t('ivr.maxRetries')}</Label>
                <Input type="number" value={menuForm.max_retries} onChange={e => setMenuForm({ ...menuForm, max_retries: Number(e.target.value) })} />
              </div>
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label>{t('ivr.timeoutAction')}</Label>
                <Select value={menuForm.timeout_action ?? ''} onValueChange={v => setMenuForm({ ...menuForm, timeout_action: v ?? '' })}>
                  <SelectTrigger className="w-full"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    {TIMEOUT_ACTIONS.map(a => <SelectItem key={a} value={a}>{actionLabel(t, a)}</SelectItem>)}
                  </SelectContent>
                </Select>
              </div>
              <div className="space-y-2">
                <Label>{t('ivr.invalidAction')}</Label>
                <Select value={menuForm.invalid_action ?? ''} onValueChange={v => setMenuForm({ ...menuForm, invalid_action: v ?? '' })}>
                  <SelectTrigger className="w-full"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    {TIMEOUT_ACTIONS.map(a => <SelectItem key={a} value={a}>{actionLabel(t, a)}</SelectItem>)}
                  </SelectContent>
                </Select>
              </div>
            </div>
            <div className="flex items-center gap-2">
              <Switch checked={menuForm.is_root} onCheckedChange={val => setMenuForm({ ...menuForm, is_root: val })} />
              <Label>{t('ivr.isRoot')}</Label>
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label>{t('ivr.startTime')}</Label>
                <Input type="time" value={menuForm.business_hours_start} onChange={e => setMenuForm({ ...menuForm, business_hours_start: e.target.value })} />
              </div>
              <div className="space-y-2">
                <Label>{t('ivr.endTime')}</Label>
                <Input type="time" value={menuForm.business_hours_end} onChange={e => setMenuForm({ ...menuForm, business_hours_end: e.target.value })} />
              </div>
            </div>
            <div className="space-y-2">
              <Label>{t('ivr.businessDays')}</Label>
              <div className="flex gap-2 flex-wrap">
                {DAYS.map(d => (
                  <label key={d.key} className="flex items-center gap-1 text-sm">
                    <Checkbox checked={selectedDays.includes(d.key)} onCheckedChange={() => toggleDay(d.key)} />
                    {d.label}
                  </label>
                ))}
              </div>
            </div>
            <div className="space-y-2">
              <Label>{t('ivr.afterHours')}</Label>
              <Select value={menuForm.after_hours_action ?? ''} onValueChange={v => setMenuForm({ ...menuForm, after_hours_action: v ?? '' })}>
                <SelectTrigger className="w-full"><SelectValue /></SelectTrigger>
                <SelectContent>
                  {TIMEOUT_ACTIONS.map(a => <SelectItem key={a} value={a}>{actionLabel(t, a)}</SelectItem>)}
                </SelectContent>
              </Select>
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setMenuDialogOpen(false)}>Cancel</Button>
              <Button type="submit">{editingMenu ? t('ivr.editMenu') : t('ivr.addMenu')}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* Option Dialog */}
      <Dialog open={optDialogOpen} onOpenChange={setOptDialogOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{editingOpt ? t('ivr.addOption') : t('ivr.addOption')}</DialogTitle>
            <DialogDescription>{t('ivr.addOption')}</DialogDescription>
          </DialogHeader>
          <form onSubmit={submitOpt} className="space-y-4">
            <div className="grid grid-cols-2 gap-4">
              <div className="space-y-2">
                <Label>{t('ivr.digit')}</Label>
                <Select value={optForm.digit ?? ''} onValueChange={v => setOptForm({ ...optForm, digit: v ?? '' })}>
                  <SelectTrigger className="w-full"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    {DIGITS.map(d => <SelectItem key={d} value={d}>{d}</SelectItem>)}
                  </SelectContent>
                </Select>
              </div>
              <div className="space-y-2">
                <Label>{t('ivr.label')}</Label>
                <Input value={optForm.label} onChange={e => setOptForm({ ...optForm, label: e.target.value })} required />
              </div>
            </div>
            <div className="space-y-2">
              <Label>{t('ivr.actionType')}</Label>
              <Select value={optForm.action_type ?? ''} onValueChange={v => setOptForm({ ...optForm, action_type: v ?? '' })}>
                <SelectTrigger className="w-full"><SelectValue /></SelectTrigger>
                <SelectContent>
                  {ACTION_TYPES.map(a => <SelectItem key={a} value={a}>{actionLabel(t, a)}</SelectItem>)}
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-2">
              <Label>{t('ivr.actionTarget')}</Label>
              <Input value={optForm.action_target} onChange={e => setOptForm({ ...optForm, action_target: e.target.value })} />
            </div>
            <div className="space-y-2">
              <Label>{t('ivr.announcement')}</Label>
              <Input value={optForm.announcement} onChange={e => setOptForm({ ...optForm, announcement: e.target.value })} />
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setOptDialogOpen(false)}>Cancel</Button>
              <Button type="submit">{t('ivr.addOption')}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
    </div>
  );
}
