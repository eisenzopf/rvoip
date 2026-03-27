import { useQuery } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { HeartPulse, Clock, Tag, Monitor } from 'lucide-react';
import { fetchHealth } from '@/lib/api';

function formatUptime(totalSecs: number): string {
  const days = Math.floor(totalSecs / 86400);
  const hours = Math.floor((totalSecs % 86400) / 3600);
  const minutes = Math.floor((totalSecs % 3600) / 60);
  const seconds = Math.floor(totalSecs % 60);

  const parts: string[] = [];
  if (days > 0) parts.push(`${days}d`);
  if (hours > 0) parts.push(`${hours}h`);
  if (minutes > 0) parts.push(`${minutes}m`);
  parts.push(`${seconds}s`);
  return parts.join(' ');
}

function getSystemInfo() {
  const nav = navigator;
  const conn = (nav as unknown as Record<string, unknown>)['connection'] as
    | { effectiveType?: string }
    | undefined;

  return {
    browser: nav.userAgent.split(' ').slice(-1)[0] ?? 'Unknown',
    platform: ((nav as unknown as Record<string, Record<string, string> | undefined>).userAgentData)?.platform ?? 'Unknown',
    language: nav.language,
    windowSize: `${window.innerWidth} x ${window.innerHeight}`,
    connectionType: conn?.effectiveType ?? 'unknown',
    cookiesEnabled: nav.cookieEnabled,
  };
}

export function Health() {
  const { t } = useTranslation();

  const { data } = useQuery({
    queryKey: ['health'],
    queryFn: fetchHealth,
    refetchInterval: 10000,
  });

  const isHealthy = data?.status === 'ok' || data?.status === 'healthy';
  const sysInfo = getSystemInfo();

  return (
    <div className="p-6 space-y-6">
      {/* Header */}
      <div>
        <h1 className="text-2xl font-bold tracking-tight">{t('health.title')}</h1>
        <p className="text-sm text-muted-foreground">
          {t('health.subtitle')}
        </p>
      </div>

      {/* Status Cards */}
      <div className="grid gap-4 md:grid-cols-3">
        {/* Status */}
        <Card>
          <CardHeader>
            <CardTitle className="text-sm font-semibold flex items-center gap-2">
              <HeartPulse className="size-4" />
              {t('health.status')}
            </CardTitle>
          </CardHeader>
          <CardContent>
            <div className="flex items-center gap-3">
              <div
                className={`h-4 w-4 rounded-full ${
                  isHealthy
                    ? 'bg-green-500 shadow-[0_0_8px_rgba(34,197,94,0.6)]'
                    : 'bg-red-500 shadow-[0_0_8px_rgba(239,68,68,0.6)]'
                }`}
              />
              <span className="text-xl font-semibold">
                {isHealthy ? t('health.healthy') : t('health.unhealthy')}
              </span>
            </div>
            {data?.status && (
              <p className="mt-1 text-xs text-muted-foreground font-mono">
                {t('health.raw')}: {data.status}
              </p>
            )}
          </CardContent>
        </Card>

        {/* Version */}
        <Card>
          <CardHeader>
            <CardTitle className="text-sm font-semibold flex items-center gap-2">
              <Tag className="size-4" />
              {t('health.version')}
            </CardTitle>
          </CardHeader>
          <CardContent>
            <span className="text-xl font-semibold font-mono">
              {data?.version ?? '--'}
            </span>
          </CardContent>
        </Card>

        {/* Uptime */}
        <Card>
          <CardHeader>
            <CardTitle className="text-sm font-semibold flex items-center gap-2">
              <Clock className="size-4" />
              {t('health.uptime')}
            </CardTitle>
          </CardHeader>
          <CardContent>
            <span className="text-xl font-semibold font-mono">
              {data ? formatUptime(data.uptime_secs) : '--'}
            </span>
          </CardContent>
        </Card>
      </div>

      {/* System Info */}
      <Card>
        <CardHeader>
          <CardTitle className="text-sm font-semibold flex items-center gap-2">
            <Monitor className="size-4" />
            {t('health.clientEnv')}
          </CardTitle>
        </CardHeader>
        <CardContent>
          <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-2 text-sm">
            <dt className="text-muted-foreground">{t('health.browser')}</dt>
            <dd className="font-mono text-xs">{sysInfo.browser}</dd>

            <dt className="text-muted-foreground">{t('health.platform')}</dt>
            <dd className="font-mono text-xs">{sysInfo.platform}</dd>

            <dt className="text-muted-foreground">{t('health.language')}</dt>
            <dd className="font-mono text-xs">{sysInfo.language}</dd>

            <dt className="text-muted-foreground">{t('health.windowSize')}</dt>
            <dd className="font-mono text-xs">{sysInfo.windowSize}</dd>

            <dt className="text-muted-foreground">{t('health.connectionType')}</dt>
            <dd className="font-mono text-xs">{sysInfo.connectionType}</dd>

            <dt className="text-muted-foreground">{t('health.cookiesEnabled')}</dt>
            <dd className="font-mono text-xs">{sysInfo.cookiesEnabled ? t('common.yes') : t('common.no')}</dd>
          </dl>
        </CardContent>
      </Card>
    </div>
  );
}
