import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { Card, CardContent } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Badge } from '@/components/ui/badge';
import { Checkbox } from '@/components/ui/checkbox';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog';
import { Plus, Trash2, Copy, Check, Key } from 'lucide-react';
import { fetchApiKeys, createApiKey, revokeApiKey } from '@/lib/api';
import type { ApiKeyView, ApiKeyCreatedResponse } from '@/lib/api';
import { useAuth } from '@/hooks/useAuth';

const AVAILABLE_PERMISSIONS = ['read', 'write', 'admin'];

export function ApiKeys() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const { user } = useAuth();
  const userId = user?.id ?? '';

  const [createOpen, setCreateOpen] = useState(false);
  const [revokeOpen, setRevokeOpen] = useState(false);
  const [keyToRevoke, setKeyToRevoke] = useState<ApiKeyView | null>(null);
  const [rawKey, setRawKey] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  // Create form
  const [name, setName] = useState('');
  const [permissions, setPermissions] = useState<string[]>([]);

  const { data: keys } = useQuery({
    queryKey: ['api-keys', userId],
    queryFn: () => fetchApiKeys(userId),
    enabled: !!userId,
    refetchInterval: 30000,
  });

  const apiKeys: ApiKeyView[] = keys ?? [];

  const createMutation = useMutation({
    mutationFn: (data: { name: string; permissions: string[] }) => createApiKey(userId, data),
    onSuccess: (response: { data: ApiKeyCreatedResponse }) => {
      queryClient.invalidateQueries({ queryKey: ['api-keys', userId] });
      setRawKey(response.data.raw_key);
      setName('');
      setPermissions([]);
    },
  });

  const revokeMutation = useMutation({
    mutationFn: (keyId: string) => revokeApiKey(userId, keyId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['api-keys', userId] });
      setRevokeOpen(false);
      setKeyToRevoke(null);
    },
  });

  function handleCreateSubmit(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    createMutation.mutate({ name, permissions });
  }

  function togglePermission(perm: string) {
    setPermissions((prev) =>
      prev.includes(perm) ? prev.filter((p) => p !== perm) : [...prev, perm],
    );
  }

  function openRevoke(key: ApiKeyView) {
    setKeyToRevoke(key);
    setRevokeOpen(true);
  }

  function handleCopy() {
    if (rawKey) {
      navigator.clipboard.writeText(rawKey);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    }
  }

  function handleCreateDialogChange(open: boolean) {
    setCreateOpen(open);
    if (!open) {
      setRawKey(null);
      setCopied(false);
      setName('');
      setPermissions([]);
    }
  }

  return (
    <div className="p-6 space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">{t('apiKeys.title')}</h1>
          <p className="text-sm text-muted-foreground">{t('apiKeys.subtitle')}</p>
        </div>
        <Dialog open={createOpen} onOpenChange={handleCreateDialogChange}>
          <DialogTrigger render={<Button size="sm" />}>
            <Plus className="size-4 mr-1.5" />
            {t('apiKeys.addKey')}
          </DialogTrigger>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>{t('apiKeys.addKey')}</DialogTitle>
              <DialogDescription>{t('apiKeys.subtitle')}</DialogDescription>
            </DialogHeader>

            {rawKey ? (
              <div className="space-y-4">
                <Label>{t('apiKeys.rawKey')}</Label>
                <div className="flex gap-2">
                  <Input readOnly value={rawKey} className="font-mono text-xs" />
                  <Button type="button" variant="outline" size="sm" onClick={handleCopy}>
                    {copied ? <Check className="size-4" /> : <Copy className="size-4" />}
                    <span className="ml-1.5">{copied ? t('apiKeys.copied') : ''}</span>
                  </Button>
                </div>
                <DialogFooter>
                  <Button onClick={() => handleCreateDialogChange(false)}>
                    {t('common.confirm')}
                  </Button>
                </DialogFooter>
              </div>
            ) : (
              <form onSubmit={handleCreateSubmit} className="space-y-4">
                <div className="space-y-2">
                  <Label htmlFor="key-name">{t('apiKeys.name')}</Label>
                  <Input
                    id="key-name"
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    required
                  />
                </div>
                <div className="space-y-2">
                  <Label>{t('apiKeys.permissions')}</Label>
                  <div className="flex flex-wrap gap-3">
                    {AVAILABLE_PERMISSIONS.map((perm) => (
                      <label key={perm} className="flex items-center gap-2 text-sm cursor-pointer">
                        <Checkbox
                          checked={permissions.includes(perm)}
                          onCheckedChange={() => togglePermission(perm)}
                        />
                        {perm}
                      </label>
                    ))}
                  </div>
                </div>
                <DialogFooter>
                  <Button type="button" variant="outline" onClick={() => handleCreateDialogChange(false)}>
                    {t('common.cancel')}
                  </Button>
                  <Button type="submit" disabled={createMutation.isPending}>
                    {t('apiKeys.addKey')}
                  </Button>
                </DialogFooter>
              </form>
            )}
          </DialogContent>
        </Dialog>
      </div>

      {/* Keys list */}
      {apiKeys.length === 0 ? (
        <div className="py-16 text-center text-muted-foreground text-sm">
          {t('apiKeys.noKeys')}
        </div>
      ) : (
        <div className="grid gap-4 md:grid-cols-2">
          {apiKeys.map((key) => (
            <Card key={key.id}>
              <CardContent className="p-4 space-y-3">
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-2">
                    <Key className="size-4 text-muted-foreground" />
                    <span className="font-medium text-sm">{key.name}</span>
                  </div>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-7 text-destructive hover:text-destructive"
                    onClick={() => openRevoke(key)}
                  >
                    <Trash2 className="size-3.5 mr-1" />
                    {t('apiKeys.revoke')}
                  </Button>
                </div>
                <div className="flex flex-wrap gap-1">
                  {key.permissions.map((perm) => (
                    <Badge key={perm} variant="secondary" className="text-[10px]">
                      {perm}
                    </Badge>
                  ))}
                </div>
                <div className="grid grid-cols-3 gap-2 text-xs text-muted-foreground">
                  <div>
                    <span className="block font-medium text-foreground">{t('apiKeys.createdAt')}</span>
                    {new Date(key.created_at).toLocaleDateString()}
                  </div>
                  <div>
                    <span className="block font-medium text-foreground">{t('apiKeys.lastUsed')}</span>
                    {key.last_used ? new Date(key.last_used).toLocaleString() : '-'}
                  </div>
                  <div>
                    <span className="block font-medium text-foreground">{t('apiKeys.expiresAt')}</span>
                    {key.expires_at ? new Date(key.expires_at).toLocaleDateString() : '-'}
                  </div>
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      )}

      {/* Revoke Confirmation */}
      <Dialog open={revokeOpen} onOpenChange={setRevokeOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('apiKeys.revoke')}</DialogTitle>
            <DialogDescription>{t('apiKeys.revokeConfirm')}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setRevokeOpen(false)}>
              {t('common.cancel')}
            </Button>
            <Button
              variant="destructive"
              onClick={() => { if (keyToRevoke) revokeMutation.mutate(keyToRevoke.id); }}
              disabled={revokeMutation.isPending}
            >
              {t('apiKeys.revoke')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
