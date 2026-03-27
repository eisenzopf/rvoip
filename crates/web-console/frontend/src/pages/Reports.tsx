import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table';
import {
  BarChart3,
  Phone,
  PhoneIncoming,
  PhoneOff,
  Clock,
  CheckCircle,
  Users,
  Download,
} from 'lucide-react';
import {
  fetchDailyReport,
  fetchAgentPerformance,
  fetchSummaryReport,
} from '@/lib/api';
import type {
  DailyReport,
  AgentPerformanceReport,
  SummaryReport,
} from '@/lib/api';

type Tab = 'daily' | 'agent' | 'summary';

function formatSeconds(s: number): string {
  if (s < 60) return `${Math.round(s)}s`;
  const m = Math.floor(s / 60);
  const sec = Math.round(s % 60);
  if (m < 60) return `${m}m ${sec}s`;
  const h = Math.floor(m / 60);
  return `${h}h ${m % 60}m`;
}

function todayStr(): string {
  return new Date().toISOString().slice(0, 10);
}

function toCsv(headers: string[], rows: string[][]): string {
  const escape = (v: string) => `"${v.replace(/"/g, '""')}"`;
  const head = headers.map(escape).join(',');
  const body = rows.map((r) => r.map(escape).join(',')).join('\n');
  return `${head}\n${body}`;
}

function downloadCsv(filename: string, content: string) {
  const blob = new Blob(['\uFEFF' + content], { type: 'text/csv;charset=utf-8;' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  a.click();
  URL.revokeObjectURL(url);
}

export function Reports() {
  const { t } = useTranslation();
  const [tab, setTab] = useState<Tab>('daily');

  // Daily
  const [dailyDate, setDailyDate] = useState(todayStr());
  const [dailyTrigger, setDailyTrigger] = useState(todayStr());

  // Agent performance
  const [agentStart, setAgentStart] = useState(todayStr());
  const [agentEnd, setAgentEnd] = useState(todayStr());
  const [agentTriggerStart, setAgentTriggerStart] = useState(todayStr());
  const [agentTriggerEnd, setAgentTriggerEnd] = useState(todayStr());

  // Summary
  const [sumStart, setSumStart] = useState(todayStr());
  const [sumEnd, setSumEnd] = useState(todayStr());
  const [sumTriggerStart, setSumTriggerStart] = useState(todayStr());
  const [sumTriggerEnd, setSumTriggerEnd] = useState(todayStr());

  const { data: daily, isFetching: dailyLoading } = useQuery({
    queryKey: ['report-daily', dailyTrigger],
    queryFn: () => fetchDailyReport(dailyTrigger),
    enabled: tab === 'daily',
    refetchInterval: false,
  });

  const { data: agents, isFetching: agentsLoading } = useQuery({
    queryKey: ['report-agents', agentTriggerStart, agentTriggerEnd],
    queryFn: () => fetchAgentPerformance(agentTriggerStart, agentTriggerEnd),
    enabled: tab === 'agent',
    refetchInterval: false,
  });

  const { data: summary, isFetching: summaryLoading } = useQuery({
    queryKey: ['report-summary', sumTriggerStart, sumTriggerEnd],
    queryFn: () => fetchSummaryReport(sumTriggerStart, sumTriggerEnd),
    enabled: tab === 'summary',
    refetchInterval: false,
  });

  function handleDailyGenerate(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    setDailyTrigger(dailyDate);
  }

  function handleAgentGenerate(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    setAgentTriggerStart(agentStart);
    setAgentTriggerEnd(agentEnd);
  }

  function handleSummaryGenerate(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    setSumTriggerStart(sumStart);
    setSumTriggerEnd(sumEnd);
  }

  function exportDaily(d: DailyReport) {
    const csv = toCsv(
      ['Date', 'Total Calls', 'Answered', 'Abandoned', 'Avg Duration (s)', 'Avg Wait (s)', 'SLA %'],
      [[d.date, String(d.total_calls), String(d.answered_calls), String(d.abandoned_calls), d.avg_duration_seconds.toFixed(1), d.avg_wait_seconds.toFixed(1), d.sla_percentage.toFixed(1)]],
    );
    downloadCsv(`daily-report-${d.date}.csv`, csv);
  }

  function exportAgents(list: AgentPerformanceReport[]) {
    const csv = toCsv(
      ['Agent ID', 'Agent Name', 'Total Calls', 'Avg Duration (s)', 'Total Duration (s)'],
      list.map((a) => [a.agent_id, a.agent_name, String(a.total_calls), a.avg_duration_seconds.toFixed(1), String(a.total_duration_seconds)]),
    );
    downloadCsv(`agent-performance-${agentTriggerStart}-${agentTriggerEnd}.csv`, csv);
  }

  function exportSummary(s: SummaryReport) {
    const lines: string[][] = [];
    lines.push(['Period', s.period, '', '', '']);
    lines.push(['Total Calls', String(s.total_calls), '', '', '']);
    lines.push(['Total Agents', String(s.total_agents), '', '', '']);
    lines.push(['Avg Calls/Agent', s.avg_calls_per_agent.toFixed(1), '', '', '']);
    lines.push(['Avg Duration', s.avg_duration.toFixed(1), '', '', '']);
    lines.push(['Busiest Hour', s.busiest_hour, '', '', '']);
    lines.push(['', '', '', '', '']);
    lines.push(['Top Agents', '', '', '', '']);
    lines.push(['Agent ID', 'Name', 'Calls', 'Avg Duration', 'Total Duration']);
    for (const a of s.top_agents) {
      lines.push([a.agent_id, a.agent_name, String(a.total_calls), a.avg_duration_seconds.toFixed(1), String(a.total_duration_seconds)]);
    }
    lines.push(['', '', '', '', '']);
    lines.push(['Queue Stats', '', '', '', '']);
    lines.push(['Queue', 'Calls', 'Avg Wait', 'Max Wait', 'Abandoned']);
    for (const q of s.queue_stats) {
      lines.push([q.queue_name, String(q.total_calls), q.avg_wait_seconds.toFixed(1), String(q.max_wait_seconds), String(q.abandoned)]);
    }
    const csv = lines.map((r) => r.map((v) => `"${v.replace(/"/g, '""')}"`).join(',')).join('\n');
    downloadCsv(`summary-${sumTriggerStart}-${sumTriggerEnd}.csv`, csv);
  }

  const tabs: { key: Tab; label: string }[] = [
    { key: 'daily', label: t('reports.daily') },
    { key: 'agent', label: t('reports.agentPerformance') },
    { key: 'summary', label: t('reports.summary') },
  ];

  return (
    <div className="p-6 space-y-6">
      <div>
        <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
          <BarChart3 className="size-6" />
          {t('reports.title')}
        </h1>
        <p className="text-sm text-muted-foreground">{t('reports.subtitle')}</p>
      </div>

      {/* Tabs */}
      <div className="flex gap-1 border-b">
        {tabs.map((tb) => (
          <button
            key={tb.key}
            className={`px-4 py-2 text-sm font-medium border-b-2 transition-colors ${
              tab === tb.key
                ? 'border-primary text-primary'
                : 'border-transparent text-muted-foreground hover:text-foreground'
            }`}
            onClick={() => setTab(tb.key)}
          >
            {tb.label}
          </button>
        ))}
      </div>

      {/* Daily Report */}
      {tab === 'daily' && (
        <div className="space-y-4">
          <form onSubmit={handleDailyGenerate} className="flex items-end gap-3">
            <div className="space-y-1">
              <Label>{t('reports.date')}</Label>
              <Input type="date" value={dailyDate} onChange={(e) => setDailyDate(e.target.value)} className="w-44" />
            </div>
            <Button type="submit" disabled={dailyLoading}>{t('reports.generate')}</Button>
            {daily && (
              <Button type="button" variant="outline" size="sm" onClick={() => exportDaily(daily)}>
                <Download className="size-4 mr-1" />
                {t('reports.export')}
              </Button>
            )}
          </form>

          {daily ? (
            <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-6">
              <StatCard icon={Phone} label={t('reports.totalCalls')} value={String(daily.total_calls)} />
              <StatCard icon={PhoneIncoming} label={t('reports.answered')} value={String(daily.answered_calls)} />
              <StatCard icon={PhoneOff} label={t('reports.abandoned')} value={String(daily.abandoned_calls)} />
              <StatCard icon={Clock} label={t('reports.avgDuration')} value={formatSeconds(daily.avg_duration_seconds)} />
              <StatCard icon={Clock} label={t('reports.avgWait')} value={formatSeconds(daily.avg_wait_seconds)} />
              <StatCard icon={CheckCircle} label={t('reports.sla')} value={`${daily.sla_percentage.toFixed(1)}%`} />
            </div>
          ) : (
            <p className="text-center text-muted-foreground py-12 text-sm">{t('reports.noData')}</p>
          )}
        </div>
      )}

      {/* Agent Performance */}
      {tab === 'agent' && (
        <div className="space-y-4">
          <form onSubmit={handleAgentGenerate} className="flex items-end gap-3">
            <div className="space-y-1">
              <Label>{t('reports.period')}</Label>
              <div className="flex gap-2">
                <Input type="date" value={agentStart} onChange={(e) => setAgentStart(e.target.value)} className="w-44" />
                <Input type="date" value={agentEnd} onChange={(e) => setAgentEnd(e.target.value)} className="w-44" />
              </div>
            </div>
            <Button type="submit" disabled={agentsLoading}>{t('reports.generate')}</Button>
            {agents && agents.length > 0 && (
              <Button type="button" variant="outline" size="sm" onClick={() => exportAgents(agents)}>
                <Download className="size-4 mr-1" />
                {t('reports.export')}
              </Button>
            )}
          </form>

          {agents && agents.length > 0 ? (
            <Card>
              <CardContent className="pt-4">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>{t('reports.agentName')}</TableHead>
                      <TableHead className="text-right">{t('reports.calls')}</TableHead>
                      <TableHead className="text-right">{t('reports.avgDuration')}</TableHead>
                      <TableHead className="text-right">{t('reports.totalDuration')}</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {agents.map((a) => (
                      <TableRow key={a.agent_id}>
                        <TableCell className="font-medium">{a.agent_name || a.agent_id}</TableCell>
                        <TableCell className="text-right font-mono">{a.total_calls}</TableCell>
                        <TableCell className="text-right font-mono">{formatSeconds(a.avg_duration_seconds)}</TableCell>
                        <TableCell className="text-right font-mono">{formatSeconds(a.total_duration_seconds)}</TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </CardContent>
            </Card>
          ) : (
            <p className="text-center text-muted-foreground py-12 text-sm">{t('reports.noData')}</p>
          )}
        </div>
      )}

      {/* Summary Report */}
      {tab === 'summary' && (
        <div className="space-y-4">
          <form onSubmit={handleSummaryGenerate} className="flex items-end gap-3">
            <div className="space-y-1">
              <Label>{t('reports.period')}</Label>
              <div className="flex gap-2">
                <Input type="date" value={sumStart} onChange={(e) => setSumStart(e.target.value)} className="w-44" />
                <Input type="date" value={sumEnd} onChange={(e) => setSumEnd(e.target.value)} className="w-44" />
              </div>
            </div>
            <Button type="submit" disabled={summaryLoading}>{t('reports.generate')}</Button>
            {summary && (
              <Button type="button" variant="outline" size="sm" onClick={() => exportSummary(summary)}>
                <Download className="size-4 mr-1" />
                {t('reports.export')}
              </Button>
            )}
          </form>

          {summary ? (
            <div className="space-y-4">
              {/* Overview cards */}
              <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-6">
                <StatCard icon={Phone} label={t('reports.totalCalls')} value={String(summary.total_calls)} />
                <StatCard icon={Users} label={t('reports.totalAgents')} value={String(summary.total_agents)} />
                <StatCard icon={BarChart3} label={t('reports.avgCallsPerAgent')} value={summary.avg_calls_per_agent.toFixed(1)} />
                <StatCard icon={Clock} label={t('reports.avgDuration')} value={formatSeconds(summary.avg_duration)} />
                <StatCard icon={Clock} label={t('reports.busiestHour')} value={summary.busiest_hour} />
              </div>

              {/* Top agents */}
              {summary.top_agents.length > 0 && (
                <Card>
                  <CardHeader>
                    <CardTitle className="text-sm font-semibold">{t('reports.topAgents')}</CardTitle>
                  </CardHeader>
                  <CardContent>
                    <Table>
                      <TableHeader>
                        <TableRow>
                          <TableHead>{t('reports.agentName')}</TableHead>
                          <TableHead className="text-right">{t('reports.calls')}</TableHead>
                          <TableHead className="text-right">{t('reports.avgDuration')}</TableHead>
                          <TableHead className="text-right">{t('reports.totalDuration')}</TableHead>
                        </TableRow>
                      </TableHeader>
                      <TableBody>
                        {summary.top_agents.map((a) => (
                          <TableRow key={a.agent_id}>
                            <TableCell className="font-medium">{a.agent_name || a.agent_id}</TableCell>
                            <TableCell className="text-right font-mono">{a.total_calls}</TableCell>
                            <TableCell className="text-right font-mono">{formatSeconds(a.avg_duration_seconds)}</TableCell>
                            <TableCell className="text-right font-mono">{formatSeconds(a.total_duration_seconds)}</TableCell>
                          </TableRow>
                        ))}
                      </TableBody>
                    </Table>
                  </CardContent>
                </Card>
              )}

              {/* Queue stats */}
              {summary.queue_stats.length > 0 && (
                <Card>
                  <CardHeader>
                    <CardTitle className="text-sm font-semibold">{t('reports.queueStats')}</CardTitle>
                  </CardHeader>
                  <CardContent>
                    <Table>
                      <TableHeader>
                        <TableRow>
                          <TableHead>{t('reports.queueName')}</TableHead>
                          <TableHead className="text-right">{t('reports.calls')}</TableHead>
                          <TableHead className="text-right">{t('reports.avgWait')}</TableHead>
                          <TableHead className="text-right">{t('reports.maxWait')}</TableHead>
                          <TableHead className="text-right">{t('reports.abandoned')}</TableHead>
                        </TableRow>
                      </TableHeader>
                      <TableBody>
                        {summary.queue_stats.map((q) => (
                          <TableRow key={q.queue_name}>
                            <TableCell className="font-medium">{q.queue_name}</TableCell>
                            <TableCell className="text-right font-mono">{q.total_calls}</TableCell>
                            <TableCell className="text-right font-mono">{formatSeconds(q.avg_wait_seconds)}</TableCell>
                            <TableCell className="text-right font-mono">{formatSeconds(q.max_wait_seconds)}</TableCell>
                            <TableCell className="text-right font-mono">{q.abandoned}</TableCell>
                          </TableRow>
                        ))}
                      </TableBody>
                    </Table>
                  </CardContent>
                </Card>
              )}
            </div>
          ) : (
            <p className="text-center text-muted-foreground py-12 text-sm">{t('reports.noData')}</p>
          )}
        </div>
      )}
    </div>
  );
}

interface StatCardProps {
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  value: string;
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
