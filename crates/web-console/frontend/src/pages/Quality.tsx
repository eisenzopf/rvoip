import { useState, useMemo } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Textarea } from '@/components/ui/textarea';
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from '@/components/ui/select';
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
import {
  Plus,
  Pencil,
  Trash2,
  ClipboardCheck,
  ChevronDown,
  ChevronRight,
} from 'lucide-react';
import {
  fetchQcTemplates,
  createQcTemplate,
  updateQcTemplate,
  deleteQcTemplate,
  createQcTemplateItem,
  deleteQcTemplateItem,
  fetchQcScores,
  submitQcScore,
} from '@/lib/api';
import type {
  QcTemplateView,
  QcTemplateItemView,
  QcScoreView,
} from '@/lib/api';

// ---------------------------------------------------------------------------
// Tab state (no Tabs component available, use simple buttons)
// ---------------------------------------------------------------------------
type TabId = 'templates' | 'scores';

// ---------------------------------------------------------------------------
// Template form
// ---------------------------------------------------------------------------
interface TemplateFormState {
  name: string;
  description: string;
  total_score: string;
}
const defaultTemplateForm: TemplateFormState = {
  name: '',
  description: '',
  total_score: '100',
};

// ---------------------------------------------------------------------------
// Item form
// ---------------------------------------------------------------------------
interface ItemFormState {
  category: string;
  item_name: string;
  max_score: string;
  description: string;
  position: string;
}
const defaultItemForm: ItemFormState = {
  category: '',
  item_name: '',
  max_score: '10',
  description: '',
  position: '0',
};

// ---------------------------------------------------------------------------
// Score form
// ---------------------------------------------------------------------------
interface ScoreFormState {
  call_id: string;
  agent_id: string;
  template_id: string;
  comments: string;
  items: Record<string, { score: string; comment: string }>;
}
const defaultScoreForm: ScoreFormState = {
  call_id: '',
  agent_id: '',
  template_id: '',
  comments: '',
  items: {},
};

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------
export function Quality() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();

  const [tab, setTab] = useState<TabId>('templates');

  // Template state
  const [createOpen, setCreateOpen] = useState(false);
  const [editOpen, setEditOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [editId, setEditId] = useState('');
  const [deleteId, setDeleteId] = useState('');
  const [tplForm, setTplForm] = useState<TemplateFormState>({ ...defaultTemplateForm });

  // Item state
  const [addItemOpen, setAddItemOpen] = useState(false);
  const [addItemTemplateId, setAddItemTemplateId] = useState('');
  const [itemForm, setItemForm] = useState<ItemFormState>({ ...defaultItemForm });

  // Template expand state
  const [expandedTemplates, setExpandedTemplates] = useState<Set<string>>(new Set());

  // Score state
  const [scoreOpen, setScoreOpen] = useState(false);
  const [scoreForm, setScoreForm] = useState<ScoreFormState>({ ...defaultScoreForm });

  // Queries
  const { data: templates } = useQuery<QcTemplateView[]>({
    queryKey: ['qc-templates'],
    queryFn: fetchQcTemplates,
    enabled: tab === 'templates' || tab === 'scores',
  });

  const { data: scores } = useQuery<QcScoreView[]>({
    queryKey: ['qc-scores'],
    queryFn: () => fetchQcScores(),
    enabled: tab === 'scores',
  });

  // Mutations: Templates
  const createTplMut = useMutation({
    mutationFn: (data: { name: string; description?: string; total_score?: number }) =>
      createQcTemplate(data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['qc-templates'] });
      setCreateOpen(false);
      setTplForm({ ...defaultTemplateForm });
    },
  });

  const updateTplMut = useMutation({
    mutationFn: ({ id, data }: { id: string; data: { name?: string; description?: string; total_score?: number } }) =>
      updateQcTemplate(id, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['qc-templates'] });
      setEditOpen(false);
      setTplForm({ ...defaultTemplateForm });
    },
  });

  const deleteTplMut = useMutation({
    mutationFn: (id: string) => deleteQcTemplate(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['qc-templates'] });
      setDeleteOpen(false);
    },
  });

  // Mutations: Items
  const addItemMut = useMutation({
    mutationFn: ({ templateId, data }: { templateId: string; data: { category: string; item_name: string; max_score: number; description?: string; position?: number } }) =>
      createQcTemplateItem(templateId, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['qc-templates'] });
      setAddItemOpen(false);
      setItemForm({ ...defaultItemForm });
    },
  });

  const deleteItemMut = useMutation({
    mutationFn: ({ templateId, itemId }: { templateId: string; itemId: string }) =>
      deleteQcTemplateItem(templateId, itemId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['qc-templates'] });
    },
  });

  // Mutations: Scores
  const submitScoreMut = useMutation({
    mutationFn: (data: { call_id: string; agent_id: string; template_id?: string; items: { item_id?: string; score: number; comment?: string }[]; comments?: string }) =>
      submitQcScore(data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['qc-scores'] });
      setScoreOpen(false);
      setScoreForm({ ...defaultScoreForm });
    },
  });

  // Handlers: Templates
  const handleCreateTemplate = (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    if (!tplForm.name.trim()) return;
    createTplMut.mutate({
      name: tplForm.name,
      description: tplForm.description || undefined,
      total_score: parseInt(tplForm.total_score, 10) || 100,
    });
  };

  const handleUpdateTemplate = (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    updateTplMut.mutate({
      id: editId,
      data: {
        name: tplForm.name || undefined,
        description: tplForm.description || undefined,
        total_score: parseInt(tplForm.total_score, 10) || undefined,
      },
    });
  };

  const openEditTemplate = (tpl: QcTemplateView) => {
    setEditId(tpl.id);
    setTplForm({
      name: tpl.name,
      description: tpl.description ?? '',
      total_score: String(tpl.total_score),
    });
    setEditOpen(true);
  };

  // Handlers: Items
  const handleAddItem = (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    if (!itemForm.category.trim() || !itemForm.item_name.trim()) return;
    addItemMut.mutate({
      templateId: addItemTemplateId,
      data: {
        category: itemForm.category,
        item_name: itemForm.item_name,
        max_score: parseInt(itemForm.max_score, 10) || 10,
        description: itemForm.description || undefined,
        position: parseInt(itemForm.position, 10) || undefined,
      },
    });
  };

  // Handlers: Scores
  const selectedTemplate = useMemo(() => {
    if (!scoreForm.template_id || !templates) return null;
    return templates.find((t) => t.id === scoreForm.template_id) ?? null;
  }, [scoreForm.template_id, templates]);

  const handleOpenScore = () => {
    const firstTpl = templates?.[0];
    const newForm: ScoreFormState = {
      call_id: '',
      agent_id: '',
      template_id: firstTpl?.id ?? '',
      comments: '',
      items: {},
    };
    if (firstTpl) {
      for (const item of firstTpl.items) {
        newForm.items[item.id] = { score: '0', comment: '' };
      }
    }
    setScoreForm(newForm);
    setScoreOpen(true);
  };

  const handleTemplateChange = (val: string) => {
    const tplId = val ?? '';
    const tpl = templates?.find((t) => t.id === tplId);
    const items: Record<string, { score: string; comment: string }> = {};
    if (tpl) {
      for (const item of tpl.items) {
        items[item.id] = { score: '0', comment: '' };
      }
    }
    setScoreForm({ ...scoreForm, template_id: tplId, items });
  };

  const handleSubmitScore = (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    if (!scoreForm.call_id.trim() || !scoreForm.agent_id.trim()) return;
    const items = Object.entries(scoreForm.items).map(([itemId, entry]) => ({
      item_id: itemId,
      score: parseInt(entry.score, 10) || 0,
      comment: entry.comment || undefined,
    }));
    submitScoreMut.mutate({
      call_id: scoreForm.call_id,
      agent_id: scoreForm.agent_id,
      template_id: scoreForm.template_id || undefined,
      items,
      comments: scoreForm.comments || undefined,
    });
  };

  const toggleExpand = (id: string) => {
    setExpandedTemplates((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  // Group items by category
  const groupByCategory = (items: QcTemplateItemView[]): Record<string, QcTemplateItemView[]> => {
    const groups: Record<string, QcTemplateItemView[]> = {};
    for (const item of items) {
      if (!groups[item.category]) groups[item.category] = [];
      groups[item.category].push(item);
    }
    return groups;
  };

  return (
    <div className="p-6 space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">{t('quality.title')}</h1>
          <p className="text-muted-foreground text-sm">{t('quality.subtitle')}</p>
        </div>
      </div>

      {/* Tab buttons */}
      <div className="flex gap-2 border-b pb-2">
        <Button
          variant={tab === 'templates' ? 'default' : 'ghost'}
          size="sm"
          onClick={() => setTab('templates')}
        >
          {t('quality.templates')}
        </Button>
        <Button
          variant={tab === 'scores' ? 'default' : 'ghost'}
          size="sm"
          onClick={() => setTab('scores')}
        >
          {t('quality.scores')}
        </Button>
      </div>

      {/* ===================== Templates Tab ===================== */}
      {tab === 'templates' && (
        <div className="space-y-4">
          <div className="flex justify-end">
            <Dialog open={createOpen} onOpenChange={setCreateOpen}>
              <DialogTrigger render={<Button size="sm" />}>
                <Plus className="size-4 mr-1" />{t('quality.addTemplate')}
              </DialogTrigger>
              <DialogContent>
                <DialogHeader>
                  <DialogTitle>{t('quality.addTemplate')}</DialogTitle>
                  <DialogDescription>{t('quality.subtitle')}</DialogDescription>
                </DialogHeader>
                <form onSubmit={handleCreateTemplate} className="space-y-4">
                  <div>
                    <Label>{t('quality.templateName')}</Label>
                    <Input value={tplForm.name} onChange={(e) => setTplForm({ ...tplForm, name: e.target.value })} required />
                  </div>
                  <div>
                    <Label>{t('quality.description')}</Label>
                    <Input value={tplForm.description} onChange={(e) => setTplForm({ ...tplForm, description: e.target.value })} />
                  </div>
                  <div>
                    <Label>{t('quality.totalScore')}</Label>
                    <Input type="number" value={tplForm.total_score} onChange={(e) => setTplForm({ ...tplForm, total_score: e.target.value })} />
                  </div>
                  <DialogFooter>
                    <Button type="button" variant="outline" onClick={() => setCreateOpen(false)}>{t('common.cancel')}</Button>
                    <Button type="submit">{t('common.save')}</Button>
                  </DialogFooter>
                </form>
              </DialogContent>
            </Dialog>
          </div>

          {(!templates || templates.length === 0) ? (
            <Card>
              <CardContent className="py-12 text-center text-muted-foreground">{t('quality.noTemplates')}</CardContent>
            </Card>
          ) : (
            <div className="space-y-4">
              {templates.map((tpl) => {
                const expanded = expandedTemplates.has(tpl.id);
                const grouped = groupByCategory(tpl.items);
                return (
                  <Card key={tpl.id}>
                    <CardHeader className="pb-2">
                      <div className="flex items-start justify-between">
                        <div
                          className="flex items-center gap-2 cursor-pointer select-none"
                          onClick={() => toggleExpand(tpl.id)}
                        >
                          {expanded ? <ChevronDown className="size-4" /> : <ChevronRight className="size-4" />}
                          <ClipboardCheck className="size-4 text-primary" />
                          <CardTitle className="text-base">{tpl.name}</CardTitle>
                          <Badge variant="secondary" className="text-xs">{tpl.total_score} {t('quality.totalScore')}</Badge>
                          <Badge variant="outline" className="text-xs">{tpl.items.length} {t('quality.items')}</Badge>
                        </div>
                        <div className="flex gap-1">
                          <Button variant="ghost" size="icon-xs" onClick={() => openEditTemplate(tpl)}>
                            <Pencil className="size-3.5" />
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon-xs"
                            onClick={() => { setDeleteId(tpl.id); setDeleteOpen(true); }}
                          >
                            <Trash2 className="size-3.5 text-destructive" />
                          </Button>
                        </div>
                      </div>
                      {tpl.description && (
                        <CardDescription>{tpl.description}</CardDescription>
                      )}
                    </CardHeader>

                    {expanded && (
                      <CardContent className="space-y-3">
                        {Object.entries(grouped).map(([category, items]) => (
                          <div key={category}>
                            <h4 className="text-sm font-semibold mb-1">{category}</h4>
                            <div className="space-y-1">
                              {items.map((item) => (
                                <div key={item.id} className="flex items-center justify-between text-sm pl-4 py-1 border-l-2">
                                  <span>{item.item_name}</span>
                                  <div className="flex items-center gap-2">
                                    <Badge variant="secondary" className="text-xs">{item.max_score} {t('quality.score')}</Badge>
                                    <Button
                                      variant="ghost"
                                      size="icon-xs"
                                      onClick={() => deleteItemMut.mutate({ templateId: tpl.id, itemId: item.id })}
                                    >
                                      <Trash2 className="size-3 text-destructive" />
                                    </Button>
                                  </div>
                                </div>
                              ))}
                            </div>
                          </div>
                        ))}
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => {
                            setAddItemTemplateId(tpl.id);
                            setItemForm({ ...defaultItemForm });
                            setAddItemOpen(true);
                          }}
                        >
                          <Plus className="size-4 mr-1" />{t('quality.addItem')}
                        </Button>
                      </CardContent>
                    )}
                  </Card>
                );
              })}
            </div>
          )}

          {/* Edit template dialog */}
          <Dialog open={editOpen} onOpenChange={setEditOpen}>
            <DialogContent>
              <DialogHeader>
                <DialogTitle>{t('quality.editTemplate')}</DialogTitle>
                <DialogDescription>{editId}</DialogDescription>
              </DialogHeader>
              <form onSubmit={handleUpdateTemplate} className="space-y-4">
                <div>
                  <Label>{t('quality.templateName')}</Label>
                  <Input value={tplForm.name} onChange={(e) => setTplForm({ ...tplForm, name: e.target.value })} required />
                </div>
                <div>
                  <Label>{t('quality.description')}</Label>
                  <Input value={tplForm.description} onChange={(e) => setTplForm({ ...tplForm, description: e.target.value })} />
                </div>
                <div>
                  <Label>{t('quality.totalScore')}</Label>
                  <Input type="number" value={tplForm.total_score} onChange={(e) => setTplForm({ ...tplForm, total_score: e.target.value })} />
                </div>
                <DialogFooter>
                  <Button type="button" variant="outline" onClick={() => setEditOpen(false)}>{t('common.cancel')}</Button>
                  <Button type="submit">{t('common.save')}</Button>
                </DialogFooter>
              </form>
            </DialogContent>
          </Dialog>

          {/* Delete template confirmation */}
          <Dialog open={deleteOpen} onOpenChange={setDeleteOpen}>
            <DialogContent>
              <DialogHeader>
                <DialogTitle>{t('common.confirm')}</DialogTitle>
                <DialogDescription>{t('quality.deleteConfirm')}</DialogDescription>
              </DialogHeader>
              <DialogFooter>
                <Button variant="outline" onClick={() => setDeleteOpen(false)}>{t('common.cancel')}</Button>
                <Button variant="destructive" onClick={() => deleteTplMut.mutate(deleteId)}>{t('common.delete')}</Button>
              </DialogFooter>
            </DialogContent>
          </Dialog>

          {/* Add item dialog */}
          <Dialog open={addItemOpen} onOpenChange={setAddItemOpen}>
            <DialogContent>
              <DialogHeader>
                <DialogTitle>{t('quality.addItem')}</DialogTitle>
                <DialogDescription>{addItemTemplateId}</DialogDescription>
              </DialogHeader>
              <form onSubmit={handleAddItem} className="space-y-4">
                <div>
                  <Label>{t('quality.category')}</Label>
                  <Input value={itemForm.category} onChange={(e) => setItemForm({ ...itemForm, category: e.target.value })} required />
                </div>
                <div>
                  <Label>{t('quality.itemName')}</Label>
                  <Input value={itemForm.item_name} onChange={(e) => setItemForm({ ...itemForm, item_name: e.target.value })} required />
                </div>
                <div>
                  <Label>{t('quality.maxScore')}</Label>
                  <Input type="number" value={itemForm.max_score} onChange={(e) => setItemForm({ ...itemForm, max_score: e.target.value })} required />
                </div>
                <div>
                  <Label>{t('quality.description')}</Label>
                  <Input value={itemForm.description} onChange={(e) => setItemForm({ ...itemForm, description: e.target.value })} />
                </div>
                <div>
                  <Label>{t('quality.position')}</Label>
                  <Input type="number" value={itemForm.position} onChange={(e) => setItemForm({ ...itemForm, position: e.target.value })} />
                </div>
                <DialogFooter>
                  <Button type="button" variant="outline" onClick={() => setAddItemOpen(false)}>{t('common.cancel')}</Button>
                  <Button type="submit">{t('common.save')}</Button>
                </DialogFooter>
              </form>
            </DialogContent>
          </Dialog>
        </div>
      )}

      {/* ===================== Scores Tab ===================== */}
      {tab === 'scores' && (
        <div className="space-y-4">
          <div className="flex justify-end">
            <Button size="sm" onClick={handleOpenScore}>
              <Plus className="size-4 mr-1" />{t('quality.scoreCall')}
            </Button>
          </div>

          {(!scores || scores.length === 0) ? (
            <Card>
              <CardContent className="py-12 text-center text-muted-foreground">{t('quality.noScores')}</CardContent>
            </Card>
          ) : (
            <Card>
              <CardContent className="p-0">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>{t('quality.callId')}</TableHead>
                      <TableHead>{t('quality.agentId')}</TableHead>
                      <TableHead>{t('quality.templates')}</TableHead>
                      <TableHead>{t('quality.score')}</TableHead>
                      <TableHead>{t('quality.scorer')}</TableHead>
                      <TableHead>{t('quality.comment')}</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {scores.map((score) => (
                      <TableRow key={score.id}>
                        <TableCell className="font-mono text-xs">{score.call_id}</TableCell>
                        <TableCell>{score.agent_id}</TableCell>
                        <TableCell>{score.template_id ?? '-'}</TableCell>
                        <TableCell>
                          <Badge variant="secondary">
                            {score.total_score ?? 0} / {score.max_score ?? 100}
                          </Badge>
                        </TableCell>
                        <TableCell>{score.scorer_id}</TableCell>
                        <TableCell className="max-w-48 truncate">{score.comments ?? '-'}</TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </CardContent>
            </Card>
          )}

          {/* Score dialog */}
          <Dialog open={scoreOpen} onOpenChange={setScoreOpen}>
            <DialogContent className="max-w-2xl max-h-[80vh] overflow-y-auto">
              <DialogHeader>
                <DialogTitle>{t('quality.submitScore')}</DialogTitle>
                <DialogDescription>{t('quality.subtitle')}</DialogDescription>
              </DialogHeader>
              <form onSubmit={handleSubmitScore} className="space-y-4">
                <div className="grid grid-cols-2 gap-4">
                  <div>
                    <Label>{t('quality.callId')}</Label>
                    <Input
                      value={scoreForm.call_id}
                      onChange={(e) => setScoreForm({ ...scoreForm, call_id: e.target.value })}
                      required
                    />
                  </div>
                  <div>
                    <Label>{t('quality.agentId')}</Label>
                    <Input
                      value={scoreForm.agent_id}
                      onChange={(e) => setScoreForm({ ...scoreForm, agent_id: e.target.value })}
                      required
                    />
                  </div>
                </div>

                <div>
                  <Label>{t('quality.templates')}</Label>
                  <Select
                    value={scoreForm.template_id}
                    onValueChange={(v) => handleTemplateChange(v ?? '')}
                  >
                    <SelectTrigger>
                      <SelectValue placeholder={t('quality.templates')} />
                    </SelectTrigger>
                    <SelectContent>
                      {(templates ?? []).map((tpl) => (
                        <SelectItem key={tpl.id} value={tpl.id}>
                          {tpl.name} ({tpl.total_score})
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>

                {/* Scoring items from selected template */}
                {selectedTemplate && (
                  <div className="space-y-3 border rounded-md p-3">
                    {Object.entries(groupByCategory(selectedTemplate.items)).map(([category, items]) => (
                      <div key={category}>
                        <h4 className="text-sm font-semibold mb-2">{category}</h4>
                        {items.map((item) => {
                          const entry = scoreForm.items[item.id] ?? { score: '0', comment: '' };
                          return (
                            <div key={item.id} className="flex items-start gap-3 mb-2 pl-3">
                              <div className="flex-1">
                                <Label className="text-xs">
                                  {item.item_name} ({t('quality.maxScore')}: {item.max_score})
                                </Label>
                                <Input
                                  type="number"
                                  min={0}
                                  max={item.max_score}
                                  value={entry.score}
                                  onChange={(e) =>
                                    setScoreForm({
                                      ...scoreForm,
                                      items: {
                                        ...scoreForm.items,
                                        [item.id]: { ...entry, score: e.target.value },
                                      },
                                    })
                                  }
                                  className="h-8 text-sm"
                                />
                              </div>
                              <div className="flex-1">
                                <Label className="text-xs">{t('quality.comment')}</Label>
                                <Input
                                  value={entry.comment}
                                  onChange={(e) =>
                                    setScoreForm({
                                      ...scoreForm,
                                      items: {
                                        ...scoreForm.items,
                                        [item.id]: { ...entry, comment: e.target.value },
                                      },
                                    })
                                  }
                                  className="h-8 text-sm"
                                />
                              </div>
                            </div>
                          );
                        })}
                      </div>
                    ))}
                  </div>
                )}

                <div>
                  <Label>{t('quality.comment')}</Label>
                  <Textarea
                    value={scoreForm.comments}
                    onChange={(e) => setScoreForm({ ...scoreForm, comments: e.target.value })}
                    rows={3}
                  />
                </div>

                <DialogFooter>
                  <Button type="button" variant="outline" onClick={() => setScoreOpen(false)}>{t('common.cancel')}</Button>
                  <Button type="submit">{t('quality.submitScore')}</Button>
                </DialogFooter>
              </form>
            </DialogContent>
          </Dialog>
        </div>
      )}
    </div>
  );
}
