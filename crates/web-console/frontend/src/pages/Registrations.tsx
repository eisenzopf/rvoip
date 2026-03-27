import { useState, useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Input } from '@/components/ui/input';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table';
import { Radio, Search } from 'lucide-react';
import { fetchRegistrations } from '@/lib/api';
import type { RegistrationView } from '@/lib/api';

function transportBadge(transport: string) {
  const t = transport.toLowerCase();
  if (t === 'udp')
    return <Badge variant="secondary">{transport}</Badge>;
  if (t === 'tcp')
    return <Badge variant="default">{transport}</Badge>;
  if (t === 'tls' || t === 'wss')
    return <Badge variant="default" className="bg-green-600">{transport}</Badge>;
  if (t === 'ws')
    return <Badge variant="secondary" className="bg-blue-500/20 text-blue-600">{transport}</Badge>;
  return <Badge variant="outline">{transport}</Badge>;
}

export function Registrations() {
  const { t } = useTranslation();
  const [filter, setFilter] = useState('');

  const { data } = useQuery({
    queryKey: ['registrations'],
    queryFn: fetchRegistrations,
    refetchInterval: 5000,
  });

  const registrations: RegistrationView[] = data?.registrations ?? [];

  const filtered = useMemo(() => {
    if (!filter) return registrations;
    const q = filter.toLowerCase();
    return registrations.filter(
      (r) =>
        r.user_id.toLowerCase().includes(q) ||
        r.contacts.some(
          (c) =>
            c.uri.toLowerCase().includes(q) ||
            c.transport.toLowerCase().includes(q) ||
            c.user_agent.toLowerCase().includes(q),
        ),
    );
  }, [registrations, filter]);

  return (
    <div className="p-6 space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">{t('registrations.title')}</h1>
          <p className="text-sm text-muted-foreground">
            {t('registrations.subtitle')}
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Radio className="size-4 text-green-500" />
          <Badge variant="outline" className="font-mono text-xs">
            {t('registrations.xTotal', { count: data?.total ?? 0 })}
          </Badge>
        </div>
      </div>

      {/* Search */}
      <div className="relative max-w-sm">
        <Search className="absolute left-3 top-1/2 -translate-y-1/2 size-4 text-muted-foreground" />
        <Input
          placeholder={t('registrations.filterPlaceholder')}
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          className="pl-9"
        />
      </div>

      {/* Registrations */}
      {filtered.length === 0 ? (
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-16">
            <Radio className="size-10 text-muted-foreground/40 mb-3" />
            <p className="text-sm text-muted-foreground">
              {registrations.length === 0
                ? t('registrations.noRegistrations')
                : t('calls.noMatch')}
            </p>
          </CardContent>
        </Card>
      ) : (
        filtered.map((reg) => (
          <Card key={reg.user_id}>
            <CardHeader className="flex flex-row items-center justify-between">
              <div className="flex items-center gap-3">
                <CardTitle className="text-sm font-semibold font-mono">
                  {reg.user_id}
                </CardTitle>
                {reg.capabilities.map((cap) => (
                  <Badge key={cap} variant="secondary" className="text-[10px]">
                    {cap}
                  </Badge>
                ))}
              </div>
              <div className="flex items-center gap-3 text-xs text-muted-foreground">
                <span>Registered: {new Date(reg.registered_at).toLocaleString()}</span>
                <span>{t('registrations.expires')}: {new Date(reg.expires).toLocaleString()}</span>
              </div>
            </CardHeader>
            <CardContent className="p-0">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead className="text-xs">{t('registrations.uri')}</TableHead>
                    <TableHead className="text-xs">{t('registrations.transport')}</TableHead>
                    <TableHead className="text-xs">{t('registrations.userAgent')}</TableHead>
                    <TableHead className="text-xs text-right">{t('registrations.expires')}</TableHead>
                    <TableHead className="text-xs text-right">{t('registrations.qValue')}</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {reg.contacts.map((contact) => (
                    <TableRow key={contact.uri}>
                      <TableCell className="font-mono text-xs">{contact.uri}</TableCell>
                      <TableCell>{transportBadge(contact.transport)}</TableCell>
                      <TableCell className="text-xs text-muted-foreground">
                        {contact.user_agent}
                      </TableCell>
                      <TableCell className="text-xs text-right font-mono text-muted-foreground">
                        {new Date(contact.expires).toLocaleString()}
                      </TableCell>
                      <TableCell className="text-xs text-right font-mono">
                        {contact.q_value.toFixed(1)}
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </CardContent>
          </Card>
        ))
      )}
    </div>
  );
}
