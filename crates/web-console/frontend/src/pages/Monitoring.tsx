import { useQuery } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import {
  Activity,
  Phone,
  GitBranch,
  Users,
  UserCheck,
  ListOrdered,
  CheckCircle,
  ArrowRight,
  XCircle,
  AlertTriangle,
} from 'lucide-react';
import { fetchRealtimeStats, fetchAlerts } from '@/lib/api';
import type { AlertView } from '@/lib/api';

function severityBadge(severity: string) {
  const s = severity.toLowerCase();
  if (s === 'info')
    return <Badge variant="secondary" className="bg-blue-500/20 text-blue-600">{severity}</Badge>;
  if (s === 'warning')
    return <Badge variant="secondary" className="bg-amber-500/20 text-amber-600">{severity}</Badge>;
  if (s === 'critical')
    return <Badge variant="default" className="bg-red-700 font-bold">{severity}</Badge>;
  if (s === 'error')
    return <Badge variant="default" className="bg-red-600">{severity}</Badge>;
  return <Badge variant="outline">{severity}</Badge>;
}

function formatTimestamp(iso: string): string {
  return new Date(iso).toLocaleString();
}

interface StatCardProps {
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  value: number;
}

function StatCard({ icon: Icon, label, value }: StatCardProps) {
  return (
    <Card>
      <CardContent className="pt-4">
        <div className="flex items-center gap-3">
          <Icon className="size-5 text-muted-foreground" />
          <div>
            <p className="text-2xl font-bold font-mono">{value}</p>
            <p className="text-xs text-muted-foreground">{label}</p>
          </div>
        </div>
      </CardContent>
    </Card>
  );
}

export function Monitoring() {
  const { t } = useTranslation();

  const { data: stats } = useQuery({
    queryKey: ['realtimeStats'],
    queryFn: fetchRealtimeStats,
    refetchInterval: 3000,
  });

  const { data: alerts } = useQuery({
    queryKey: ['alerts'],
    queryFn: fetchAlerts,
    refetchInterval: 3000,
  });

  const alertList: AlertView[] = alerts ?? [];

  return (
    <div className="p-6 space-y-6">
      <div>
        <h1 className="text-2xl font-bold tracking-tight">{t('monitoring.title')}</h1>
        <p className="text-sm text-muted-foreground">{t('monitoring.subtitle')}</p>
      </div>

      {/* Real-time Stats */}
      <div>
        <h2 className="text-sm font-semibold text-muted-foreground uppercase mb-3">
          {t('monitoring.realtimeStats')}
        </h2>
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-6">
          <StatCard icon={Phone} label={t('dashboard.activeCalls')} value={stats?.active_calls ?? 0} />
          <StatCard icon={GitBranch} label="Bridges" value={stats?.active_bridges ?? 0} />
          <StatCard icon={UserCheck} label={t('dashboard.agentsOnline')} value={stats?.available_agents ?? 0} />
          <StatCard icon={Users} label={t('agents.busy')} value={stats?.busy_agents ?? 0} />
          <StatCard icon={ListOrdered} label={t('dashboard.queueDepth')} value={stats?.queued_calls ?? 0} />
          <StatCard icon={CheckCircle} label={t('dashboard.totalHandled')} value={stats?.total_calls_handled ?? 0} />
        </div>
      </div>

      {/* Routing Stats */}
      <Card>
        <CardHeader>
          <CardTitle className="text-sm font-semibold flex items-center gap-2">
            <Activity className="size-4" />
            {t('monitoring.routingStats')}
          </CardTitle>
        </CardHeader>
        <CardContent>
          <div className="grid gap-6 sm:grid-cols-3">
            <div className="flex items-center gap-3">
              <ArrowRight className="size-5 text-green-500" />
              <div>
                <p className="text-xl font-bold font-mono">{stats?.routing_stats?.calls_routed_directly ?? 0}</p>
                <p className="text-xs text-muted-foreground">{t('monitoring.directRouted')}</p>
              </div>
            </div>
            <div className="flex items-center gap-3">
              <ListOrdered className="size-5 text-amber-500" />
              <div>
                <p className="text-xl font-bold font-mono">{stats?.routing_stats?.calls_queued ?? 0}</p>
                <p className="text-xs text-muted-foreground">{t('monitoring.queued')}</p>
              </div>
            </div>
            <div className="flex items-center gap-3">
              <XCircle className="size-5 text-red-500" />
              <div>
                <p className="text-xl font-bold font-mono">{stats?.routing_stats?.calls_rejected ?? 0}</p>
                <p className="text-xs text-muted-foreground">{t('monitoring.rejected')}</p>
              </div>
            </div>
          </div>
        </CardContent>
      </Card>

      {/* Alerts */}
      <Card>
        <CardHeader>
          <CardTitle className="text-sm font-semibold flex items-center gap-2">
            <AlertTriangle className="size-4" />
            {t('monitoring.alerts')}
          </CardTitle>
        </CardHeader>
        <CardContent>
          {alertList.length === 0 ? (
            <p className="text-center text-muted-foreground py-8 text-sm">{t('monitoring.noAlerts')}</p>
          ) : (
            <div className="space-y-3">
              {alertList.map((alert) => (
                <div key={alert.id} className="flex items-start gap-3 rounded-md border p-3">
                  <div className="pt-0.5">{severityBadge(alert.severity)}</div>
                  <div className="flex-1 min-w-0">
                    <p className="text-sm">{alert.message}</p>
                    <p className="text-[10px] text-muted-foreground font-mono mt-1">
                      {formatTimestamp(alert.timestamp)}
                    </p>
                  </div>
                </div>
              ))}
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
