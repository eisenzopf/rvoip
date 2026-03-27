import { useQuery } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import {
  ChartContainer,
  ChartTooltip,
  ChartTooltipContent,
  type ChartConfig,
} from '@/components/ui/chart';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table';
import { Bar, BarChart, CartesianGrid, XAxis, YAxis } from 'recharts';
import { Phone, Users, ListOrdered, Radio, Activity, Signal } from 'lucide-react';
import {
  fetchDashboard,
  fetchCalls,
  fetchAgents,
  fetchQueues,
  fetchActivity,
} from '@/lib/api';
import { useWebSocket } from '@/hooks/useWebSocket';
import type { DashboardMetrics, ActiveCall, AgentView, QueueView } from '@/lib/api';

function StatCard({
  title,
  value,
  icon: Icon,
  accent,
}: {
  title: string;
  value: number | string;
  icon: React.ComponentType<{ className?: string }>;
  accent?: string;
}) {
  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between pb-2">
        <CardTitle className="text-sm font-medium text-muted-foreground">
          {title}
        </CardTitle>
        <Icon className={`size-4 ${accent ?? 'text-muted-foreground'}`} />
      </CardHeader>
      <CardContent>
        <div className={`text-2xl font-bold font-mono ${accent ?? ''}`}>
          {value}
        </div>
      </CardContent>
    </Card>
  );
}

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

const chartConfig = {
  calls: { label: 'Calls', color: 'var(--chart-1)' },
  queued: { label: 'Queued', color: 'var(--chart-3)' },
} satisfies ChartConfig;

export function Dashboard() {
  const { t } = useTranslation();

  const { data: metrics } = useQuery<DashboardMetrics>({
    queryKey: ['dashboard'],
    queryFn: fetchDashboard,
  });
  const { data: activityData } = useQuery({
    queryKey: ['activity'],
    queryFn: fetchActivity,
  });
  const { data: callsData } = useQuery({
    queryKey: ['calls'],
    queryFn: fetchCalls,
  });
  const { data: agentsData } = useQuery({
    queryKey: ['agents'],
    queryFn: fetchAgents,
  });
  const { data: queuesData } = useQuery({
    queryKey: ['queues'],
    queryFn: fetchQueues,
  });
  const { events, connected } = useWebSocket();

  const calls: ActiveCall[] = callsData?.calls ?? [];
  const agents: AgentView[] = agentsData?.agents ?? [];
  const queues: QueueView[] = queuesData?.queues ?? [];
  const chartData = (activityData?.hours ?? []).map((h) => ({
    hour: `${String(h.hour).padStart(2, '0')}:00`,
    calls: h.calls,
    queued: h.queued,
  }));

  return (
    <div className="p-6 space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">{t('dashboard.title')}</h1>
          <p className="text-sm text-muted-foreground font-mono">{t('dashboard.subtitle')}</p>
        </div>
        <div className="flex items-center gap-3">
          <div className="flex items-center gap-1.5 rounded-md border px-2.5 py-1 text-xs font-mono">
            <div className={`h-1.5 w-1.5 rounded-full ${connected ? 'bg-green-500 animate-pulse' : 'bg-red-500'}`} />
            {connected ? t('dashboard.live') : t('dashboard.disconnected')}
          </div>
        </div>
      </div>

      {/* KPI Cards */}
      <div className="grid gap-4 md:grid-cols-3 lg:grid-cols-5">
        <StatCard
          title={t('dashboard.activeCalls')}
          value={metrics?.active_calls ?? 0}
          icon={Phone}
          accent="text-green-500"
        />
        <StatCard
          title={t('dashboard.sipRegistrations')}
          value={metrics?.sip_registrations ?? 0}
          icon={Signal}
          accent="text-cyan-500"
        />
        <StatCard
          title={t('dashboard.agentsOnline')}
          value={`${metrics?.available_agents ?? 0} / ${(metrics?.available_agents ?? 0) + (metrics?.busy_agents ?? 0)}`}
          icon={Users}
          accent="text-blue-500"
        />
        <StatCard
          title={t('dashboard.queueDepth')}
          value={metrics?.queued_calls ?? 0}
          icon={ListOrdered}
          accent="text-amber-500"
        />
        <StatCard
          title={t('dashboard.totalHandled')}
          value={metrics?.total_calls_handled ?? 0}
          icon={Activity}
          accent="text-purple-500"
        />
      </div>

      {/* Call Activity Chart */}
      <Card>
        <CardHeader>
          <CardTitle className="text-sm font-semibold">{t('dashboard.callActivity24h')}</CardTitle>
        </CardHeader>
        <CardContent>
          {chartData.length === 0 ? (
            <p className="text-center text-muted-foreground text-sm py-12">{t('dashboard.noActivityData')}</p>
          ) : (
            <ChartContainer config={chartConfig} className="h-[200px] w-full">
              <BarChart data={chartData} accessibilityLayer>
                <CartesianGrid vertical={false} strokeDasharray="3 3" />
                <XAxis
                  dataKey="hour"
                  tickLine={false}
                  axisLine={false}
                  tickMargin={8}
                  fontSize={10}
                />
                <YAxis
                  tickLine={false}
                  axisLine={false}
                  width={32}
                  fontSize={10}
                />
                <ChartTooltip content={<ChartTooltipContent />} />
                <Bar dataKey="calls" fill="var(--color-calls)" radius={[3, 3, 0, 0]} />
                <Bar dataKey="queued" fill="var(--color-queued)" radius={[3, 3, 0, 0]} />
              </BarChart>
            </ChartContainer>
          )}
        </CardContent>
      </Card>

      {/* Main Grid */}
      <div className="grid gap-4 lg:grid-cols-2">
        {/* Active Calls Table */}
        <Card>
          <CardHeader className="flex flex-row items-center justify-between">
            <CardTitle className="text-sm font-semibold">{t('dashboard.calls')}</CardTitle>
            <Badge variant="outline" className="font-mono text-xs">
              {t('dashboard.xCalls', { count: calls.length })}
            </Badge>
          </CardHeader>
          <CardContent className="p-0">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="text-xs">{t('calls.from')}</TableHead>
                  <TableHead className="text-xs">{t('calls.to')}</TableHead>
                  <TableHead className="text-xs">{t('calls.status')}</TableHead>
                  <TableHead className="text-xs">{t('calls.agent')}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {calls.length === 0 ? (
                  <TableRow>
                    <TableCell colSpan={4} className="text-center text-muted-foreground text-sm py-8">
                      {t('dashboard.noCalls')}
                    </TableCell>
                  </TableRow>
                ) : (
                  calls.slice(0, 8).map((call) => (
                    <TableRow key={call.call_id}>
                      <TableCell className="font-mono text-xs">{call.from_uri}</TableCell>
                      <TableCell className="font-mono text-xs">{call.to_uri}</TableCell>
                      <TableCell>{statusBadge(call.status)}</TableCell>
                      <TableCell className="text-xs">{call.agent_id ?? '—'}</TableCell>
                    </TableRow>
                  ))
                )}
              </TableBody>
            </Table>
          </CardContent>
        </Card>

        {/* Agents */}
        <Card>
          <CardHeader className="flex flex-row items-center justify-between">
            <CardTitle className="text-sm font-semibold">{t('dashboard.agents')}</CardTitle>
            <Badge variant="outline" className="font-mono text-xs">
              {t('dashboard.xOnline', { count: agentsData?.online ?? 0 })}
            </Badge>
          </CardHeader>
          <CardContent>
            {agents.length === 0 ? (
              <p className="text-center text-muted-foreground text-sm py-8">
                {t('dashboard.noAgents')}
              </p>
            ) : (
              <div className="grid gap-2 sm:grid-cols-2">
                {agents.slice(0, 6).map((agent) => (
                  <div
                    key={agent.id}
                    className="flex items-center gap-3 rounded-lg border p-3"
                  >
                    <div className="relative flex h-8 w-8 items-center justify-center rounded-md bg-muted text-xs font-semibold">
                      {agent.id.slice(0, 2).toUpperCase()}
                      <span
                        className={`absolute -bottom-0.5 -right-0.5 h-2.5 w-2.5 rounded-full border-2 border-background ${
                          agent.status === 'Available'
                            ? 'bg-green-500'
                            : agent.status === 'Offline'
                              ? 'bg-muted-foreground'
                              : 'bg-amber-500'
                        }`}
                      />
                    </div>
                    <div className="flex-1 min-w-0">
                      <p className="text-xs font-medium truncate">{agent.id}</p>
                      <p className="text-[11px] text-muted-foreground font-mono">
                        {agent.status} &middot; {agent.current_calls}/{agent.max_calls} calls
                      </p>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </CardContent>
        </Card>

        {/* Queues */}
        <Card>
          <CardHeader>
            <CardTitle className="text-sm font-semibold">{t('dashboard.queues')}</CardTitle>
          </CardHeader>
          <CardContent>
            {queues.length === 0 ? (
              <p className="text-center text-muted-foreground text-sm py-8">
                {t('dashboard.noQueues')}
              </p>
            ) : (
              <div className="space-y-4">
                {queues.map((q) => (
                  <div key={q.queue_id} className="space-y-1.5">
                    <div className="flex justify-between text-xs">
                      <span className="font-medium">{q.queue_id}</span>
                      <span className="text-muted-foreground font-mono">
                        {q.total_calls} waiting &middot; avg {q.avg_wait_secs}s
                      </span>
                    </div>
                    <div className="h-1.5 rounded-full bg-muted overflow-hidden">
                      <div
                        className="h-full rounded-full bg-amber-500 transition-all"
                        style={{ width: `${Math.min(q.total_calls * 10, 100)}%` }}
                      />
                    </div>
                  </div>
                ))}
              </div>
            )}
          </CardContent>
        </Card>

        {/* Event Stream */}
        <Card>
          <CardHeader className="flex flex-row items-center justify-between">
            <CardTitle className="text-sm font-semibold">{t('dashboard.eventStream')}</CardTitle>
            <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
              <Radio className="size-3" />
              {t('dashboard.xEvents', { count: events.length })}
            </div>
          </CardHeader>
          <CardContent>
            {events.length === 0 ? (
              <p className="text-center text-muted-foreground text-sm py-8">
                {t('dashboard.waitingForEvents')}
              </p>
            ) : (
              <div className="space-y-1 max-h-[300px] overflow-y-auto">
                {events.slice(0, 20).map((evt, i) => (
                  <div
                    key={i}
                    className="flex items-center gap-2 rounded px-2 py-1 text-xs hover:bg-muted/50"
                  >
                    <span className="text-muted-foreground font-mono w-16 shrink-0">
                      {new Date(evt.timestamp).toLocaleTimeString()}
                    </span>
                    <Badge variant="outline" className="text-[10px] shrink-0">
                      {evt.event_type}
                    </Badge>
                    <span className="text-muted-foreground truncate">
                      {typeof evt.data === 'string' ? evt.data : JSON.stringify(evt.data)}
                    </span>
                  </div>
                ))}
              </div>
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
