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
import { ScrollText } from 'lucide-react';
import { fetchAuditLog } from '@/lib/api';
import type { AuditLogEntry } from '@/lib/api';

const PAGE_SIZE = 20;

function actionBadge(action: string) {
  const a = action.toLowerCase();
  if (a.includes('create') || a.includes('add'))
    return <Badge variant="default" className="bg-green-600">{action}</Badge>;
  if (a.includes('delete') || a.includes('remove'))
    return <Badge variant="default" className="bg-red-600">{action}</Badge>;
  if (a.includes('update') || a.includes('edit'))
    return <Badge variant="secondary" className="bg-blue-500/20 text-blue-600">{action}</Badge>;
  if (a.includes('login') || a.includes('auth'))
    return <Badge variant="secondary" className="bg-amber-500/20 text-amber-600">{action}</Badge>;
  return <Badge variant="outline">{action}</Badge>;
}

function truncateJson(val: unknown): string {
  if (val === null || val === undefined) return '--';
  const s = typeof val === 'string' ? val : JSON.stringify(val);
  return s.length > 80 ? s.slice(0, 80) + '...' : s;
}

function formatDateTime(iso: string): string {
  return new Date(iso).toLocaleString();
}

export function AuditLog() {
  const { t } = useTranslation();
  const [offset, setOffset] = useState(0);
  const [accumulated, setAccumulated] = useState<AuditLogEntry[]>([]);

  const { data, isFetching } = useQuery({
    queryKey: ['auditLog', offset],
    queryFn: () => fetchAuditLog({ limit: PAGE_SIZE, offset }),
  });

  const currentData = data ?? [];
  const displayEntries = offset === 0 ? currentData : [...accumulated, ...currentData];

  const handleLoadMore = () => {
    setAccumulated(displayEntries);
    setOffset(offset + PAGE_SIZE);
  };

  return (
    <div className="p-6 space-y-6">
      <div>
        <h1 className="text-2xl font-bold tracking-tight">{t('audit.title')}</h1>
        <p className="text-sm text-muted-foreground">{t('audit.subtitle')}</p>
      </div>

      <Card>
        <CardHeader>
          <CardTitle className="text-sm font-semibold flex items-center gap-2">
            <ScrollText className="size-4" />
            {t('audit.title')}
          </CardTitle>
        </CardHeader>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="text-xs">{t('audit.time')}</TableHead>
                <TableHead className="text-xs">{t('audit.user')}</TableHead>
                <TableHead className="text-xs">{t('audit.action')}</TableHead>
                <TableHead className="text-xs">{t('audit.resource')}</TableHead>
                <TableHead className="text-xs">{t('audit.details')}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {displayEntries.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={5} className="text-center text-muted-foreground text-sm py-16">
                    {t('audit.noLogs')}
                  </TableCell>
                </TableRow>
              ) : (
                displayEntries.map((entry) => (
                  <TableRow key={entry.id}>
                    <TableCell className="text-xs font-mono text-muted-foreground whitespace-nowrap">
                      {formatDateTime(entry.created_at)}
                    </TableCell>
                    <TableCell className="text-xs">{entry.username}</TableCell>
                    <TableCell>{actionBadge(entry.action)}</TableCell>
                    <TableCell className="text-xs font-mono">
                      {entry.resource_type}
                      {entry.resource_id ? `/${entry.resource_id}` : ''}
                    </TableCell>
                    <TableCell className="text-xs font-mono text-muted-foreground max-w-[200px] truncate" title={typeof entry.details === 'string' ? entry.details : JSON.stringify(entry.details)}>
                      {truncateJson(entry.details)}
                    </TableCell>
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
            {t('audit.loadMore')}
          </Button>
        </div>
      )}
    </div>
  );
}
