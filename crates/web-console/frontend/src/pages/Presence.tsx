import { useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { CircleDot } from 'lucide-react';
import { fetchPresence, updateMyPresence } from '@/lib/api';
import type { PresenceView } from '@/lib/api';

const STATUS_OPTIONS = ['available', 'busy', 'away', 'dnd', 'offline'] as const;

function statusDot(status: string) {
  const s = status.toLowerCase();
  if (s === 'available') return 'bg-green-500';
  if (s === 'busy') return 'bg-red-500';
  if (s === 'away') return 'bg-amber-500';
  if (s === 'dnd') return 'bg-red-400';
  return 'bg-gray-400';
}

function statusLabel(status: string, t: (key: string) => string): string {
  const s = status.toLowerCase();
  if (s === 'available') return t('presence.available');
  if (s === 'busy') return t('presence.busy');
  if (s === 'away') return t('presence.away');
  if (s === 'dnd') return t('presence.dnd');
  if (s === 'offline') return t('presence.offline');
  return status;
}

function formatDateTime(iso: string): string {
  return new Date(iso).toLocaleString();
}

function initials(userId: string): string {
  return userId.slice(0, 2).toUpperCase();
}

export function Presence() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [myStatus, setMyStatus] = useState('available');
  const [myNote, setMyNote] = useState('');

  const { data: users } = useQuery({
    queryKey: ['presence'],
    queryFn: fetchPresence,
    refetchInterval: 5000,
  });

  const presenceList: PresenceView[] = users ?? [];

  const handleUpdate = async (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    try {
      await updateMyPresence(myStatus, myNote || undefined);
      queryClient.invalidateQueries({ queryKey: ['presence'] });
    } catch {
      // update failed
    }
  };

  return (
    <div className="p-6 space-y-6">
      <div>
        <h1 className="text-2xl font-bold tracking-tight">{t('presence.title')}</h1>
        <p className="text-sm text-muted-foreground">{t('presence.subtitle')}</p>
      </div>

      {/* My Status */}
      <Card>
        <CardHeader>
          <CardTitle className="text-sm font-semibold flex items-center gap-2">
            <CircleDot className="size-4" />
            {t('presence.myStatus')}
          </CardTitle>
        </CardHeader>
        <CardContent>
          <form onSubmit={handleUpdate} className="flex items-end gap-3">
            <div className="space-y-1">
              <select
                className="h-9 rounded-md border bg-background px-3 text-sm focus:outline-none focus:ring-2 focus:ring-ring"
                value={myStatus}
                onChange={(e) => setMyStatus(e.target.value)}
              >
                {STATUS_OPTIONS.map((s) => (
                  <option key={s} value={s}>{statusLabel(s, t)}</option>
                ))}
              </select>
            </div>
            <div className="flex-1">
              <Input
                placeholder="Note..."
                value={myNote}
                onChange={(e) => setMyNote(e.target.value)}
              />
            </div>
            <Button type="submit" size="sm">{t('presence.updateStatus')}</Button>
          </form>
        </CardContent>
      </Card>

      {/* Users Grid */}
      {presenceList.length === 0 ? (
        <div className="text-center text-muted-foreground py-16">{t('presence.noUsers')}</div>
      ) : (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
          {presenceList.map((user) => (
            <Card key={user.user_id}>
              <CardContent className="pt-4">
                <div className="flex items-center gap-3">
                  <div className="relative">
                    <div className="flex h-10 w-10 items-center justify-center rounded-full bg-muted text-sm font-semibold">
                      {initials(user.user_id)}
                    </div>
                    <div className={`absolute -bottom-0.5 -right-0.5 h-3 w-3 rounded-full border-2 border-background ${statusDot(user.status)}`} />
                  </div>
                  <div className="min-w-0 flex-1">
                    <p className="text-sm font-medium truncate">{user.user_id}</p>
                    <p className="text-xs text-muted-foreground">{statusLabel(user.status, t)}</p>
                  </div>
                </div>
                {user.note && (
                  <p className="mt-2 text-xs text-muted-foreground italic truncate">{user.note}</p>
                )}
                <p className="mt-1 text-[10px] text-muted-foreground font-mono">
                  {formatDateTime(user.last_updated)}
                </p>
              </CardContent>
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}
