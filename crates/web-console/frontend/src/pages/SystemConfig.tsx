import { useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog';
import { Settings, Download, Upload } from 'lucide-react';
import { fetchSystemConfig, exportConfig, importConfig } from '@/lib/api';

export function SystemConfig() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [importJson, setImportJson] = useState('');
  const [importOpen, setImportOpen] = useState(false);
  const [message, setMessage] = useState('');

  const { data: config } = useQuery({
    queryKey: ['systemConfig'],
    queryFn: fetchSystemConfig,
  });

  const handleExport = async () => {
    try {
      const raw = await exportConfig();
      const blob = new Blob([typeof raw === 'string' ? raw : JSON.stringify(raw, null, 2)], { type: 'application/json' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = 'rvoip-config.json';
      a.click();
      URL.revokeObjectURL(url);
      setMessage(t('systemConfig.exported'));
      setTimeout(() => setMessage(''), 3000);
    } catch {
      // export failed silently
    }
  };

  const handleImport = async (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    if (!importJson.trim()) return;
    try {
      await importConfig(importJson);
      setMessage(t('systemConfig.imported'));
      setImportJson('');
      setImportOpen(false);
      queryClient.invalidateQueries({ queryKey: ['systemConfig'] });
      setTimeout(() => setMessage(''), 3000);
    } catch {
      // import failed silently
    }
  };

  return (
    <div className="p-6 space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">{t('systemConfig.title')}</h1>
          <p className="text-sm text-muted-foreground">{t('systemConfig.subtitle')}</p>
        </div>
        <div className="flex items-center gap-2">
          <Button variant="outline" size="sm" onClick={handleExport}>
            <Download className="size-4 mr-1.5" />
            {t('systemConfig.export')}
          </Button>
          <Dialog open={importOpen} onOpenChange={setImportOpen}>
            <DialogTrigger render={
              <Button variant="outline" size="sm">
                <Upload className="size-4 mr-1.5" />
                {t('systemConfig.import')}
              </Button>
            } />
            <DialogContent>
              <DialogHeader>
                <DialogTitle>{t('systemConfig.import')}</DialogTitle>
                <DialogDescription>{t('systemConfig.subtitle')}</DialogDescription>
              </DialogHeader>
              <form onSubmit={handleImport} className="space-y-4">
                <textarea
                  className="w-full h-48 rounded-md border bg-muted p-3 font-mono text-xs resize-none focus:outline-none focus:ring-2 focus:ring-ring"
                  placeholder={t('systemConfig.importPlaceholder')}
                  value={importJson}
                  onChange={(e) => setImportJson(e.target.value)}
                />
                <div className="flex justify-end gap-2">
                  <Button type="button" variant="outline" onClick={() => setImportOpen(false)}>
                    {t('common.cancel')}
                  </Button>
                  <Button type="submit">{t('systemConfig.import')}</Button>
                </div>
              </form>
            </DialogContent>
          </Dialog>
        </div>
      </div>

      {message && (
        <div className="rounded-md bg-green-500/10 border border-green-500/30 px-4 py-2 text-sm text-green-700 dark:text-green-400">
          {message}
        </div>
      )}

      <Card>
        <CardHeader>
          <CardTitle className="text-sm font-semibold flex items-center gap-2">
            <Settings className="size-4" />
            {t('systemConfig.currentConfig')}
          </CardTitle>
        </CardHeader>
        <CardContent>
          <pre className="overflow-auto max-h-[600px] rounded-md bg-muted p-4 font-mono text-xs">
            {config ? JSON.stringify(config, null, 2) : '--'}
          </pre>
        </CardContent>
      </Card>
    </div>
  );
}
