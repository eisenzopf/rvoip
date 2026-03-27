import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Textarea } from '@/components/ui/textarea';
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
import { Plus, Pencil, Trash2, BookOpen, Eye, Copy, FileText, MessageSquare } from 'lucide-react';
import {
  fetchArticles,
  createArticle,
  updateArticle,
  deleteArticle,
  viewArticle,
  fetchScripts,
  createScript,
  updateScript,
  deleteScript,
} from '@/lib/api';
import type { ArticleView, ScriptView } from '@/lib/api';

// -- Article form state -------------------------------------------------------

interface ArticleFormState {
  title: string;
  category: string;
  content: string;
  tags: string;
  is_published: boolean;
}

const defaultArticleForm: ArticleFormState = {
  title: '',
  category: '',
  content: '',
  tags: '',
  is_published: false,
};

// -- Script form state --------------------------------------------------------

interface ScriptFormState {
  name: string;
  scenario: string;
  content: string;
  category: string;
  is_active: boolean;
}

const defaultScriptForm: ScriptFormState = {
  name: '',
  scenario: '',
  content: '',
  category: '',
  is_active: true,
};

export function Knowledge() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();

  const [tab, setTab] = useState<'articles' | 'scripts'>('articles');
  const [search, setSearch] = useState('');
  const [categoryFilter, setCategoryFilter] = useState('');
  const [copiedId, setCopiedId] = useState<string | null>(null);

  // Article dialogs
  const [createArticleOpen, setCreateArticleOpen] = useState(false);
  const [editArticleOpen, setEditArticleOpen] = useState(false);
  const [deleteArticleOpen, setDeleteArticleOpen] = useState(false);
  const [editArticleId, setEditArticleId] = useState('');
  const [deleteArticleId, setDeleteArticleId] = useState('');
  const [articleForm, setArticleForm] = useState<ArticleFormState>({ ...defaultArticleForm });

  // Script dialogs
  const [createScriptOpen, setCreateScriptOpen] = useState(false);
  const [editScriptOpen, setEditScriptOpen] = useState(false);
  const [deleteScriptOpen, setDeleteScriptOpen] = useState(false);
  const [editScriptId, setEditScriptId] = useState('');
  const [deleteScriptId, setDeleteScriptId] = useState('');
  const [scriptForm, setScriptForm] = useState<ScriptFormState>({ ...defaultScriptForm });

  // Queries
  const { data: articles } = useQuery<ArticleView[]>({
    queryKey: ['articles', categoryFilter, search],
    queryFn: () => fetchArticles({
      category: categoryFilter || undefined,
      search: search || undefined,
    }),
    enabled: tab === 'articles',
  });

  const { data: scripts } = useQuery<ScriptView[]>({
    queryKey: ['scripts', categoryFilter],
    queryFn: () => fetchScripts({
      category: tab === 'scripts' ? (categoryFilter || undefined) : undefined,
    }),
    enabled: tab === 'scripts',
  });

  // Article mutations
  const createArticleMut = useMutation({
    mutationFn: (data: { title: string; category?: string; content: string; tags?: string; is_published?: boolean }) =>
      createArticle(data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['articles'] });
      setCreateArticleOpen(false);
      setArticleForm({ ...defaultArticleForm });
    },
  });

  const updateArticleMut = useMutation({
    mutationFn: ({ id, data }: { id: string; data: { title?: string; category?: string; content?: string; tags?: string; is_published?: boolean } }) =>
      updateArticle(id, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['articles'] });
      setEditArticleOpen(false);
      setArticleForm({ ...defaultArticleForm });
    },
  });

  const deleteArticleMut = useMutation({
    mutationFn: (id: string) => deleteArticle(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['articles'] });
      setDeleteArticleOpen(false);
    },
  });

  // Script mutations
  const createScriptMut = useMutation({
    mutationFn: (data: { name: string; scenario?: string; content: string; category?: string; is_active?: boolean }) =>
      createScript(data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['scripts'] });
      setCreateScriptOpen(false);
      setScriptForm({ ...defaultScriptForm });
    },
  });

  const updateScriptMut = useMutation({
    mutationFn: ({ id, data }: { id: string; data: { name?: string; scenario?: string; content?: string; category?: string; is_active?: boolean } }) =>
      updateScript(id, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['scripts'] });
      setEditScriptOpen(false);
      setScriptForm({ ...defaultScriptForm });
    },
  });

  const deleteScriptMut = useMutation({
    mutationFn: (id: string) => deleteScript(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['scripts'] });
      setDeleteScriptOpen(false);
    },
  });

  // Handlers
  const handleCreateArticle = (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    if (!articleForm.title.trim() || !articleForm.content.trim()) return;
    createArticleMut.mutate({
      title: articleForm.title,
      category: articleForm.category || undefined,
      content: articleForm.content,
      tags: articleForm.tags || undefined,
      is_published: articleForm.is_published,
    });
  };

  const handleUpdateArticle = (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    updateArticleMut.mutate({
      id: editArticleId,
      data: {
        title: articleForm.title || undefined,
        category: articleForm.category || undefined,
        content: articleForm.content || undefined,
        tags: articleForm.tags || undefined,
        is_published: articleForm.is_published,
      },
    });
  };

  const openEditArticle = (article: ArticleView) => {
    setEditArticleId(article.id);
    setArticleForm({
      title: article.title,
      category: article.category ?? '',
      content: article.content,
      tags: article.tags ?? '',
      is_published: article.is_published,
    });
    setEditArticleOpen(true);
  };

  const handleCreateScript = (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    if (!scriptForm.name.trim() || !scriptForm.content.trim()) return;
    createScriptMut.mutate({
      name: scriptForm.name,
      scenario: scriptForm.scenario || undefined,
      content: scriptForm.content,
      category: scriptForm.category || undefined,
      is_active: scriptForm.is_active,
    });
  };

  const handleUpdateScript = (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    updateScriptMut.mutate({
      id: editScriptId,
      data: {
        name: scriptForm.name || undefined,
        scenario: scriptForm.scenario || undefined,
        content: scriptForm.content || undefined,
        category: scriptForm.category || undefined,
        is_active: scriptForm.is_active,
      },
    });
  };

  const openEditScript = (script: ScriptView) => {
    setEditScriptId(script.id);
    setScriptForm({
      name: script.name,
      scenario: script.scenario ?? '',
      content: script.content,
      category: script.category ?? '',
      is_active: script.is_active,
    });
    setEditScriptOpen(true);
  };

  const handleCopyScript = (script: ScriptView) => {
    navigator.clipboard.writeText(script.content).then(() => {
      setCopiedId(script.id);
      setTimeout(() => setCopiedId(null), 2000);
    });
  };

  const handleViewArticle = (id: string) => {
    viewArticle(id);
  };

  // Get unique categories from current data
  const articleCategories = [...new Set((articles ?? []).map(a => a.category).filter(Boolean))] as string[];
  const scriptCategories = [...new Set((scripts ?? []).map(s => s.category).filter(Boolean))] as string[];

  return (
    <div className="p-6 space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">{t('knowledge.title')}</h1>
          <p className="text-muted-foreground text-sm">{t('knowledge.subtitle')}</p>
        </div>
      </div>

      {/* Tabs */}
      <div className="flex gap-2 border-b">
        <button
          className={`px-4 py-2 text-sm font-medium border-b-2 transition-colors ${
            tab === 'articles'
              ? 'border-primary text-primary'
              : 'border-transparent text-muted-foreground hover:text-foreground'
          }`}
          onClick={() => { setTab('articles'); setCategoryFilter(''); }}
        >
          <FileText className="size-4 inline mr-1" />
          {t('knowledge.articles')}
        </button>
        <button
          className={`px-4 py-2 text-sm font-medium border-b-2 transition-colors ${
            tab === 'scripts'
              ? 'border-primary text-primary'
              : 'border-transparent text-muted-foreground hover:text-foreground'
          }`}
          onClick={() => { setTab('scripts'); setCategoryFilter(''); }}
        >
          <MessageSquare className="size-4 inline mr-1" />
          {t('knowledge.scripts')}
        </button>
      </div>

      {/* Articles Tab */}
      {tab === 'articles' && (
        <>
          <div className="flex items-center gap-3">
            <Input
              placeholder={t('knowledge.search')}
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              className="max-w-xs"
            />
            <select
              className="h-9 rounded-md border border-input bg-background px-3 text-sm"
              value={categoryFilter}
              onChange={(e) => setCategoryFilter(e.target.value)}
            >
              <option value="">{t('knowledge.category')}</option>
              {articleCategories.map((c) => (
                <option key={c} value={c}>{c}</option>
              ))}
            </select>
            <div className="ml-auto">
              <Dialog open={createArticleOpen} onOpenChange={setCreateArticleOpen}>
                <DialogTrigger render={<Button size="sm" />}>
                  <Plus className="size-4 mr-1" />{t('knowledge.addArticle')}
                </DialogTrigger>
                <DialogContent className="max-w-2xl">
                  <DialogHeader>
                    <DialogTitle>{t('knowledge.addArticle')}</DialogTitle>
                    <DialogDescription>{t('knowledge.subtitle')}</DialogDescription>
                  </DialogHeader>
                  <form onSubmit={handleCreateArticle} className="space-y-4">
                    <div>
                      <Label>{t('knowledge.articleTitle')}</Label>
                      <Input value={articleForm.title} onChange={(e) => setArticleForm({ ...articleForm, title: e.target.value })} required />
                    </div>
                    <div>
                      <Label>{t('knowledge.category')}</Label>
                      <Input value={articleForm.category} onChange={(e) => setArticleForm({ ...articleForm, category: e.target.value })} />
                    </div>
                    <div>
                      <Label>{t('knowledge.content')}</Label>
                      <Textarea
                        value={articleForm.content}
                        onChange={(e) => setArticleForm({ ...articleForm, content: e.target.value })}
                        rows={8}
                        required
                      />
                    </div>
                    <div>
                      <Label>{t('knowledge.tags')}</Label>
                      <Input
                        value={articleForm.tags}
                        onChange={(e) => setArticleForm({ ...articleForm, tags: e.target.value })}
                        placeholder="tag1, tag2, tag3"
                      />
                    </div>
                    <div className="flex items-center gap-2">
                      <Switch
                        checked={articleForm.is_published}
                        onCheckedChange={(v) => setArticleForm({ ...articleForm, is_published: v })}
                      />
                      <Label>{t('knowledge.published')}</Label>
                    </div>
                    <DialogFooter>
                      <Button type="button" variant="outline" onClick={() => setCreateArticleOpen(false)}>{t('common.cancel')}</Button>
                      <Button type="submit">{t('common.save')}</Button>
                    </DialogFooter>
                  </form>
                </DialogContent>
              </Dialog>
            </div>
          </div>

          {(!articles || articles.length === 0) ? (
            <Card>
              <CardContent className="py-12 text-center text-muted-foreground">{t('knowledge.noArticles')}</CardContent>
            </Card>
          ) : (
            <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
              {articles.map((article) => (
                <Card key={article.id} className="cursor-pointer" onClick={() => handleViewArticle(article.id)}>
                  <CardHeader className="pb-2">
                    <div className="flex items-start justify-between">
                      <div className="flex items-center gap-2 min-w-0">
                        <BookOpen className="size-4 text-primary shrink-0" />
                        <CardTitle className="text-base truncate">{article.title}</CardTitle>
                      </div>
                      <div className="flex gap-1 shrink-0">
                        <Button variant="ghost" size="icon-xs" onClick={(e) => { e.stopPropagation(); openEditArticle(article); }}>
                          <Pencil className="size-3.5" />
                        </Button>
                        <Button variant="ghost" size="icon-xs" onClick={(e) => { e.stopPropagation(); setDeleteArticleId(article.id); setDeleteArticleOpen(true); }}>
                          <Trash2 className="size-3.5 text-destructive" />
                        </Button>
                      </div>
                    </div>
                    <div className="flex items-center gap-2 mt-1">
                      {article.category && (
                        <Badge variant="secondary" className="text-xs">{article.category}</Badge>
                      )}
                      <Badge variant={article.is_published ? 'default' : 'outline'} className="text-xs">
                        {article.is_published ? t('knowledge.published') : t('knowledge.draft')}
                      </Badge>
                    </div>
                  </CardHeader>
                  <CardContent>
                    {article.tags && (
                      <div className="flex flex-wrap gap-1 mb-2">
                        {article.tags.split(',').map((tag) => (
                          <Badge key={tag.trim()} variant="outline" className="text-[10px]">{tag.trim()}</Badge>
                        ))}
                      </div>
                    )}
                    <div className="flex items-center justify-between text-xs text-muted-foreground mt-2">
                      <div className="flex items-center gap-1">
                        <Eye className="size-3" />
                        <span>{article.view_count}</span>
                      </div>
                      <span>{new Date(article.updated_at).toLocaleDateString()}</span>
                    </div>
                  </CardContent>
                </Card>
              ))}
            </div>
          )}
        </>
      )}

      {/* Scripts Tab */}
      {tab === 'scripts' && (
        <>
          <div className="flex items-center gap-3">
            <select
              className="h-9 rounded-md border border-input bg-background px-3 text-sm"
              value={categoryFilter}
              onChange={(e) => setCategoryFilter(e.target.value)}
            >
              <option value="">{t('knowledge.category')}</option>
              {scriptCategories.map((c) => (
                <option key={c} value={c}>{c}</option>
              ))}
            </select>
            <div className="ml-auto">
              <Dialog open={createScriptOpen} onOpenChange={setCreateScriptOpen}>
                <DialogTrigger render={<Button size="sm" />}>
                  <Plus className="size-4 mr-1" />{t('knowledge.addScript')}
                </DialogTrigger>
                <DialogContent className="max-w-2xl">
                  <DialogHeader>
                    <DialogTitle>{t('knowledge.addScript')}</DialogTitle>
                    <DialogDescription>{t('knowledge.subtitle')}</DialogDescription>
                  </DialogHeader>
                  <form onSubmit={handleCreateScript} className="space-y-4">
                    <div>
                      <Label>{t('knowledge.scriptName')}</Label>
                      <Input value={scriptForm.name} onChange={(e) => setScriptForm({ ...scriptForm, name: e.target.value })} required />
                    </div>
                    <div>
                      <Label>{t('knowledge.scenario')}</Label>
                      <Input value={scriptForm.scenario} onChange={(e) => setScriptForm({ ...scriptForm, scenario: e.target.value })} />
                    </div>
                    <div>
                      <Label>{t('knowledge.content')}</Label>
                      <Textarea
                        value={scriptForm.content}
                        onChange={(e) => setScriptForm({ ...scriptForm, content: e.target.value })}
                        rows={8}
                        required
                      />
                    </div>
                    <div>
                      <Label>{t('knowledge.category')}</Label>
                      <Input value={scriptForm.category} onChange={(e) => setScriptForm({ ...scriptForm, category: e.target.value })} />
                    </div>
                    <div className="flex items-center gap-2">
                      <Switch
                        checked={scriptForm.is_active}
                        onCheckedChange={(v) => setScriptForm({ ...scriptForm, is_active: v })}
                      />
                      <Label>{t('knowledge.active')}</Label>
                    </div>
                    <DialogFooter>
                      <Button type="button" variant="outline" onClick={() => setCreateScriptOpen(false)}>{t('common.cancel')}</Button>
                      <Button type="submit">{t('common.save')}</Button>
                    </DialogFooter>
                  </form>
                </DialogContent>
              </Dialog>
            </div>
          </div>

          {(!scripts || scripts.length === 0) ? (
            <Card>
              <CardContent className="py-12 text-center text-muted-foreground">{t('knowledge.noScripts')}</CardContent>
            </Card>
          ) : (
            <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
              {scripts.map((script) => (
                <Card key={script.id}>
                  <CardHeader className="pb-2">
                    <div className="flex items-start justify-between">
                      <div className="flex items-center gap-2 min-w-0">
                        <MessageSquare className="size-4 text-primary shrink-0" />
                        <CardTitle className="text-base truncate">{script.name}</CardTitle>
                      </div>
                      <div className="flex gap-1 shrink-0">
                        <Button
                          variant="ghost"
                          size="icon-xs"
                          onClick={() => handleCopyScript(script)}
                          title={t('knowledge.copy')}
                        >
                          <Copy className="size-3.5" />
                        </Button>
                        <Button variant="ghost" size="icon-xs" onClick={() => openEditScript(script)}>
                          <Pencil className="size-3.5" />
                        </Button>
                        <Button variant="ghost" size="icon-xs" onClick={() => { setDeleteScriptId(script.id); setDeleteScriptOpen(true); }}>
                          <Trash2 className="size-3.5 text-destructive" />
                        </Button>
                      </div>
                    </div>
                    <div className="flex items-center gap-2 mt-1">
                      {script.scenario && (
                        <Badge variant="secondary" className="text-xs">{script.scenario}</Badge>
                      )}
                      <Badge variant={script.is_active ? 'default' : 'outline'} className="text-xs">
                        {script.is_active ? t('knowledge.active') : t('knowledge.draft')}
                      </Badge>
                    </div>
                  </CardHeader>
                  <CardContent>
                    <p className="text-sm text-muted-foreground line-clamp-3 whitespace-pre-line">
                      {script.content}
                    </p>
                    {copiedId === script.id && (
                      <p className="text-xs text-green-600 mt-2">{t('knowledge.copied')}</p>
                    )}
                  </CardContent>
                </Card>
              ))}
            </div>
          )}
        </>
      )}

      {/* Edit Article Dialog */}
      <Dialog open={editArticleOpen} onOpenChange={setEditArticleOpen}>
        <DialogContent className="max-w-2xl">
          <DialogHeader>
            <DialogTitle>{t('knowledge.editArticle')}</DialogTitle>
            <DialogDescription>{editArticleId}</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleUpdateArticle} className="space-y-4">
            <div>
              <Label>{t('knowledge.articleTitle')}</Label>
              <Input value={articleForm.title} onChange={(e) => setArticleForm({ ...articleForm, title: e.target.value })} required />
            </div>
            <div>
              <Label>{t('knowledge.category')}</Label>
              <Input value={articleForm.category} onChange={(e) => setArticleForm({ ...articleForm, category: e.target.value })} />
            </div>
            <div>
              <Label>{t('knowledge.content')}</Label>
              <Textarea
                value={articleForm.content}
                onChange={(e) => setArticleForm({ ...articleForm, content: e.target.value })}
                rows={8}
                required
              />
            </div>
            <div>
              <Label>{t('knowledge.tags')}</Label>
              <Input
                value={articleForm.tags}
                onChange={(e) => setArticleForm({ ...articleForm, tags: e.target.value })}
                placeholder="tag1, tag2, tag3"
              />
            </div>
            <div className="flex items-center gap-2">
              <Switch
                checked={articleForm.is_published}
                onCheckedChange={(v) => setArticleForm({ ...articleForm, is_published: v })}
              />
              <Label>{t('knowledge.published')}</Label>
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setEditArticleOpen(false)}>{t('common.cancel')}</Button>
              <Button type="submit">{t('common.save')}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* Delete Article Confirmation */}
      <Dialog open={deleteArticleOpen} onOpenChange={setDeleteArticleOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('common.confirm')}</DialogTitle>
            <DialogDescription>{t('knowledge.deleteConfirm')}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteArticleOpen(false)}>{t('common.cancel')}</Button>
            <Button variant="destructive" onClick={() => deleteArticleMut.mutate(deleteArticleId)}>{t('common.delete')}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Edit Script Dialog */}
      <Dialog open={editScriptOpen} onOpenChange={setEditScriptOpen}>
        <DialogContent className="max-w-2xl">
          <DialogHeader>
            <DialogTitle>{t('knowledge.editScript')}</DialogTitle>
            <DialogDescription>{editScriptId}</DialogDescription>
          </DialogHeader>
          <form onSubmit={handleUpdateScript} className="space-y-4">
            <div>
              <Label>{t('knowledge.scriptName')}</Label>
              <Input value={scriptForm.name} onChange={(e) => setScriptForm({ ...scriptForm, name: e.target.value })} required />
            </div>
            <div>
              <Label>{t('knowledge.scenario')}</Label>
              <Input value={scriptForm.scenario} onChange={(e) => setScriptForm({ ...scriptForm, scenario: e.target.value })} />
            </div>
            <div>
              <Label>{t('knowledge.content')}</Label>
              <Textarea
                value={scriptForm.content}
                onChange={(e) => setScriptForm({ ...scriptForm, content: e.target.value })}
                rows={8}
                required
              />
            </div>
            <div>
              <Label>{t('knowledge.category')}</Label>
              <Input value={scriptForm.category} onChange={(e) => setScriptForm({ ...scriptForm, category: e.target.value })} />
            </div>
            <div className="flex items-center gap-2">
              <Switch
                checked={scriptForm.is_active}
                onCheckedChange={(v) => setScriptForm({ ...scriptForm, is_active: v })}
              />
              <Label>{t('knowledge.active')}</Label>
            </div>
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setEditScriptOpen(false)}>{t('common.cancel')}</Button>
              <Button type="submit">{t('common.save')}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* Delete Script Confirmation */}
      <Dialog open={deleteScriptOpen} onOpenChange={setDeleteScriptOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('common.confirm')}</DialogTitle>
            <DialogDescription>{t('knowledge.deleteConfirm')}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteScriptOpen(false)}>{t('common.cancel')}</Button>
            <Button variant="destructive" onClick={() => deleteScriptMut.mutate(deleteScriptId)}>{t('common.delete')}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
