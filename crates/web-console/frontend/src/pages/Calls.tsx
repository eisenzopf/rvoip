import { useState, useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Input } from '@/components/ui/input';
import { Separator } from '@/components/ui/separator';
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table';
import { Phone, Search } from 'lucide-react';
import { fetchCalls } from '@/lib/api';
import type { ActiveCall } from '@/lib/api';

function statusBadge(status: string) {
  const s = status.toLowerCase();
  if (s.includes('active') || s.includes('connected'))
    return <Badge variant="default" className="bg-green-600">{status}</Badge>;
  if (s.includes('queued') || s.includes('waiting'))
    return <Badge variant="secondary" className="bg-amber-500/20 text-amber-600">{status}</Badge>;
  if (s.includes('ringing'))
    return <Badge variant="secondary" className="bg-blue-500/20 text-blue-600">{status}</Badge>;
  return <Badge variant="outline">{status}</Badge>;
}

function formatTime(iso: string) {
  return new Date(iso).toLocaleTimeString();
}

function formatDateTime(iso: string | null) {
  if (!iso) return '—';
  return new Date(iso).toLocaleString();
}

function DetailRow({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="flex justify-between py-1.5 text-sm">
      <span className="text-muted-foreground">{label}</span>
      <span className="font-mono text-xs text-right max-w-[60%] break-all">{value}</span>
    </div>
  );
}

export function Calls() {
  const { t } = useTranslation();
  const [filter, setFilter] = useState('');
  const [selected, setSelected] = useState<ActiveCall | null>(null);

  const { data } = useQuery({
    queryKey: ['calls'],
    queryFn: fetchCalls,
    refetchInterval: 5000,
  });

  const calls: ActiveCall[] = data?.calls ?? [];

  const filtered = useMemo(() => {
    if (!filter) return calls;
    const q = filter.toLowerCase();
    return calls.filter(
      (c) =>
        c.call_id.toLowerCase().includes(q) ||
        c.from_uri.toLowerCase().includes(q) ||
        c.to_uri.toLowerCase().includes(q) ||
        (c.agent_id?.toLowerCase().includes(q) ?? false) ||
        c.status.toLowerCase().includes(q),
    );
  }, [calls, filter]);

  return (
    <div className="p-6 space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">{t('calls.title')}</h1>
          <p className="text-sm text-muted-foreground">
            {t('calls.totalActive', { count: data?.total ?? 0 })}
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Phone className="size-4 text-green-500" />
          <span className="text-sm font-mono font-medium">{t('calls.shown', { count: filtered.length })}</span>
        </div>
      </div>

      {/* Search */}
      <div className="relative max-w-sm">
        <Search className="absolute left-3 top-1/2 -translate-y-1/2 size-4 text-muted-foreground" />
        <Input
          placeholder={t('calls.filterPlaceholder')}
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          className="pl-9"
        />
      </div>

      {/* Table */}
      <Card>
        <CardHeader className="flex flex-row items-center justify-between">
          <CardTitle className="text-sm font-semibold">{t('calls.activeCalls')}</CardTitle>
          <Badge variant="outline" className="font-mono text-xs">
            {t('calls.autoRefresh')}
          </Badge>
        </CardHeader>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="text-xs">{t('calls.callId')}</TableHead>
                <TableHead className="text-xs">{t('calls.from')}</TableHead>
                <TableHead className="text-xs">{t('calls.to')}</TableHead>
                <TableHead className="text-xs">{t('calls.agent')}</TableHead>
                <TableHead className="text-xs">{t('calls.status')}</TableHead>
                <TableHead className="text-xs text-right">{t('calls.priority')}</TableHead>
                <TableHead className="text-xs text-right">{t('calls.created')}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {filtered.length === 0 ? (
                <TableRow>
                  <TableCell
                    colSpan={7}
                    className="text-center text-muted-foreground text-sm py-16"
                  >
                    {calls.length === 0
                      ? t('calls.noActiveCalls')
                      : t('calls.noMatch')}
                  </TableCell>
                </TableRow>
              ) : (
                filtered.map((call) => (
                  <TableRow
                    key={call.call_id}
                    className="cursor-pointer"
                    onClick={() => setSelected(call)}
                  >
                    <TableCell className="font-mono text-xs max-w-[140px] truncate">
                      {call.call_id}
                    </TableCell>
                    <TableCell className="font-mono text-xs">{call.from_uri}</TableCell>
                    <TableCell className="font-mono text-xs">{call.to_uri}</TableCell>
                    <TableCell className="text-xs">{call.agent_id ?? '—'}</TableCell>
                    <TableCell>{statusBadge(call.status)}</TableCell>
                    <TableCell className="text-xs text-right font-mono">
                      {call.priority}
                    </TableCell>
                    <TableCell className="text-xs text-right font-mono text-muted-foreground">
                      {formatTime(call.created_at)}
                    </TableCell>
                  </TableRow>
                ))
              )}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      {/* Call Detail Sheet */}
      <Sheet open={!!selected} onOpenChange={(open) => { if (!open) setSelected(null); }}>
        <SheetContent>
          <SheetHeader>
            <SheetTitle>{t('calls.detail.title')}</SheetTitle>
            <SheetDescription className="font-mono text-xs">
              {selected?.call_id}
            </SheetDescription>
          </SheetHeader>

          {selected && (
            <div className="mt-4 space-y-4">
              {/* Status */}
              <div className="flex items-center gap-2">
                {statusBadge(selected.status)}
                <span className="text-xs text-muted-foreground">
                  {t('calls.priority')} {selected.priority}
                </span>
              </div>

              <Separator />

              {/* SIP Info */}
              <div>
                <h4 className="text-xs font-semibold uppercase text-muted-foreground mb-2">
                  {t('calls.detail.sipDetails')}
                </h4>
                <DetailRow label={t('calls.from')} value={selected.from_uri} />
                <DetailRow label={t('calls.to')} value={selected.to_uri} />
                <DetailRow label={t('calls.detail.callerId')} value={selected.caller_id} />
                <DetailRow label={t('calls.detail.customerType')} value={selected.customer_type} />
              </div>

              <Separator />

              {/* Routing */}
              <div>
                <h4 className="text-xs font-semibold uppercase text-muted-foreground mb-2">
                  {t('calls.detail.routing')}
                </h4>
                <DetailRow label={t('calls.agent')} value={selected.agent_id ?? '—'} />
                <DetailRow label={t('calls.detail.queue')} value={selected.queue_id ?? '—'} />
                {selected.required_skills.length > 0 && (
                  <div className="flex justify-between py-1.5 text-sm">
                    <span className="text-muted-foreground">{t('calls.detail.skills')}</span>
                    <div className="flex gap-1 flex-wrap justify-end">
                      {selected.required_skills.map((s) => (
                        <Badge key={s} variant="outline" className="text-[10px]">
                          {s}
                        </Badge>
                      ))}
                    </div>
                  </div>
                )}
              </div>

              <Separator />

              {/* Timeline */}
              <div>
                <h4 className="text-xs font-semibold uppercase text-muted-foreground mb-2">
                  {t('calls.detail.timeline')}
                </h4>
                <DetailRow label={t('calls.detail.created')} value={formatDateTime(selected.created_at)} />
                <DetailRow label={t('calls.detail.queued')} value={formatDateTime(selected.queued_at)} />
                <DetailRow label={t('calls.detail.answered')} value={formatDateTime(selected.answered_at)} />
                <DetailRow label={t('calls.detail.ended')} value={formatDateTime(selected.ended_at)} />
              </div>
            </div>
          )}
        </SheetContent>
      </Sheet>
    </div>
  );
}
