import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table';
import { Clock } from 'lucide-react';
import { fetchCallHistory } from '@/lib/api';
import type { CallHistoryEntry } from '@/lib/api';

const PAGE_SIZE = 20;

function formatDuration(seconds: number | null): string {
  if (seconds === null || seconds === undefined) return '--';
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  return m > 0 ? `${m}m ${s}s` : `${s}s`;
}

function dispositionBadge(disposition: string | null) {
  if (!disposition) return <span className="text-muted-foreground">--</span>;
  const d = disposition.toLowerCase();
  if (d === 'answered')
    return <Badge variant="default" className="bg-green-600">{disposition}</Badge>;
  if (d === 'abandoned')
    return <Badge variant="default" className="bg-red-600">{disposition}</Badge>;
  if (d === 'timeout')
    return <Badge variant="secondary" className="bg-amber-500/20 text-amber-600">{disposition}</Badge>;
  if (d === 'error')
    return <Badge variant="default" className="bg-red-600">{disposition}</Badge>;
  return <Badge variant="outline">{disposition}</Badge>;
}

function formatDateTime(iso: string | null): string {
  if (!iso) return '--';
  return new Date(iso).toLocaleString();
}

export function CallHistory() {
  const { t } = useTranslation();
  const [offset, setOffset] = useState(0);
  const [allEntries, setAllEntries] = useState<CallHistoryEntry[]>([]);

  const { data, isFetching } = useQuery({
    queryKey: ['callHistory', offset],
    queryFn: () => fetchCallHistory({ limit: PAGE_SIZE, offset }),
  });

  const entries = offset === 0 ? (data ?? []) : allEntries;
  const currentData = data ?? [];

  const handleLoadMore = () => {
    const newOffset = offset + PAGE_SIZE;
    setAllEntries([...entries, ...currentData]);
    setOffset(newOffset);
  };

  const displayEntries = offset === 0 ? currentData : [...allEntries.slice(0, offset), ...currentData];

  return (
    <div className="p-6 space-y-6">
      <div>
        <h1 className="text-2xl font-bold tracking-tight">{t('callHistory.title')}</h1>
        <p className="text-sm text-muted-foreground">{t('callHistory.subtitle')}</p>
      </div>

      <Card>
        <CardHeader>
          <CardTitle className="text-sm font-semibold flex items-center gap-2">
            <Clock className="size-4" />
            {t('callHistory.title')}
          </CardTitle>
        </CardHeader>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="text-xs">{t('callHistory.callId')}</TableHead>
                <TableHead className="text-xs">{t('callHistory.customer')}</TableHead>
                <TableHead className="text-xs">{t('callHistory.agent')}</TableHead>
                <TableHead className="text-xs">{t('callHistory.queue')}</TableHead>
                <TableHead className="text-xs">{t('callHistory.startTime')}</TableHead>
                <TableHead className="text-xs">{t('callHistory.duration')}</TableHead>
                <TableHead className="text-xs">{t('callHistory.disposition')}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {displayEntries.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={7} className="text-center text-muted-foreground text-sm py-16">
                    {t('callHistory.noRecords')}
                  </TableCell>
                </TableRow>
              ) : (
                displayEntries.map((entry) => (
                  <TableRow key={entry.call_id}>
                    <TableCell className="font-mono text-xs max-w-[140px] truncate">{entry.call_id}</TableCell>
                    <TableCell className="text-xs">{entry.customer_number ?? '--'}</TableCell>
                    <TableCell className="text-xs">{entry.agent_id ?? '--'}</TableCell>
                    <TableCell className="text-xs">{entry.queue_name ?? '--'}</TableCell>
                    <TableCell className="text-xs font-mono text-muted-foreground">{formatDateTime(entry.start_time)}</TableCell>
                    <TableCell className="text-xs font-mono">{formatDuration(entry.duration_seconds)}</TableCell>
                    <TableCell>{dispositionBadge(entry.disposition)}</TableCell>
                  </TableRow>
                ))
              )}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      {currentData.length >= PAGE_SIZE && (
        <div className="flex justify-center">
          <Button variant="outline" onClick={handleLoadMore} disabled={isFetching}>
            {t('callHistory.loadMore')}
          </Button>
        </div>
      )}
    </div>
  );
}
