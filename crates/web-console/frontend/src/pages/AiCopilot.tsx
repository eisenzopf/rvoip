import { useState, useRef, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Textarea } from '@/components/ui/textarea';
import { BrainCircuit, Copy, Check, BookOpen, Loader2 } from 'lucide-react';
import { analyzeText } from '@/lib/api';
import type { AnalyzeResponse } from '@/lib/api';

function sentimentColor(value: number): string {
  if (value < -0.5) return 'bg-red-500';
  if (value < -0.1) return 'bg-orange-400';
  if (value > 0.3) return 'bg-green-500';
  return 'bg-gray-400';
}

function sentimentPercent(value: number): number {
  // Map -1..1 to 0..100
  return Math.round((value + 1) * 50);
}

export function AiCopilot() {
  const { t } = useTranslation();
  const [text, setText] = useState('');
  const [loading, setLoading] = useState(false);
  const [result, setResult] = useState<AnalyzeResponse | null>(null);
  const [copied, setCopied] = useState(false);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const handleAnalyze = async (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    if (!text.trim() || loading) return;
    setLoading(true);
    setResult(null);
    try {
      const res = await analyzeText(text.trim());
      if (res.code === 200 && res.data) {
        setResult(res.data);
      }
    } finally {
      setLoading(false);
    }
  };

  const handleCopy = async (content: string) => {
    await navigator.clipboard.writeText(content);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <div className="p-6 space-y-6 max-w-4xl mx-auto">
      <div className="flex items-center gap-3">
        <BrainCircuit className="size-6 text-primary" />
        <h1 className="text-2xl font-bold">{t('copilot.title')}</h1>
      </div>

      {/* Input form */}
      <Card>
        <CardContent className="pt-6">
          <form onSubmit={handleAnalyze} className="space-y-4">
            <Textarea
              ref={inputRef}
              value={text}
              onChange={(e) => setText(e.target.value)}
              placeholder={t('copilot.inputPlaceholder')}
              rows={4}
              className="resize-none"
            />
            <div className="flex justify-end">
              <Button type="submit" disabled={!text.trim() || loading}>
                {loading ? (
                  <>
                    <Loader2 className="size-4 mr-2 animate-spin" />
                    {t('copilot.analyzing')}
                  </>
                ) : (
                  <>
                    <BrainCircuit className="size-4 mr-2" />
                    {t('copilot.analyze')}
                  </>
                )}
              </Button>
            </div>
          </form>
        </CardContent>
      </Card>

      {/* Results */}
      {!result && !loading && (
        <div className="text-center text-muted-foreground py-12">
          {t('copilot.noResult')}
        </div>
      )}

      {result && (
        <div className="grid gap-4 md:grid-cols-2">
          {/* Intent & Sentiment */}
          <Card>
            <CardHeader className="pb-3">
              <CardTitle className="text-sm font-medium">{t('copilot.intent')}</CardTitle>
            </CardHeader>
            <CardContent>
              <Badge variant="secondary" className="text-sm">
                {t(`copilot.intentLabels.${result.intent}`, result.intent)}
              </Badge>
            </CardContent>
          </Card>

          <Card>
            <CardHeader className="pb-3">
              <CardTitle className="text-sm font-medium">{t('copilot.sentiment')}</CardTitle>
            </CardHeader>
            <CardContent className="space-y-2">
              <div className="flex items-center gap-2">
                <Badge variant={result.sentiment < -0.1 ? 'destructive' : result.sentiment > 0.3 ? 'default' : 'secondary'}>
                  {t(`copilot.sentimentLabels.${result.sentiment_label}`, result.sentiment_label)}
                </Badge>
                <span className="text-xs text-muted-foreground">
                  ({result.sentiment.toFixed(1)})
                </span>
              </div>
              <div className="w-full bg-muted rounded-full h-2">
                <div
                  className={`h-2 rounded-full transition-all ${sentimentColor(result.sentiment)}`}
                  style={{ width: `${sentimentPercent(result.sentiment)}%` }}
                />
              </div>
            </CardContent>
          </Card>

          {/* Suggested Reply */}
          <Card className="md:col-span-2">
            <CardHeader className="pb-3">
              <div className="flex items-center justify-between">
                <CardTitle className="text-sm font-medium">{t('copilot.suggestion')}</CardTitle>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => handleCopy(result.suggestion)}
                >
                  {copied ? (
                    <Check className="size-4 mr-1" />
                  ) : (
                    <Copy className="size-4 mr-1" />
                  )}
                  {t('copilot.copy')}
                </Button>
              </div>
            </CardHeader>
            <CardContent>
              <div className="bg-muted/50 rounded-lg p-4 text-sm whitespace-pre-wrap">
                {result.suggestion}
              </div>
            </CardContent>
          </Card>

          {/* Knowledge References */}
          {result.knowledge_refs.length > 0 && (
            <Card>
              <CardHeader className="pb-3">
                <CardTitle className="text-sm font-medium flex items-center gap-2">
                  <BookOpen className="size-4" />
                  {t('copilot.knowledge')}
                </CardTitle>
              </CardHeader>
              <CardContent>
                <ul className="space-y-2">
                  {result.knowledge_refs.map((ref_item) => (
                    <li key={ref_item.id} className="flex items-center justify-between text-sm">
                      <span>{ref_item.title}</span>
                      <Badge variant="outline" className="text-xs">
                        {Math.round(ref_item.relevance * 100)}%
                      </Badge>
                    </li>
                  ))}
                </ul>
              </CardContent>
            </Card>
          )}

          {/* Quality Checklist */}
          <Card>
            <CardHeader className="pb-3">
              <CardTitle className="text-sm font-medium">{t('copilot.quality')}</CardTitle>
            </CardHeader>
            <CardContent>
              <ul className="space-y-2">
                {result.quality_items.map((item) => (
                  <li key={item.name} className="flex items-center gap-2 text-sm">
                    <input
                      type="checkbox"
                      checked={item.checked}
                      readOnly
                      className="rounded border-gray-300"
                    />
                    <span className={item.checked ? 'text-foreground' : 'text-muted-foreground'}>
                      {item.name}
                    </span>
                  </li>
                ))}
              </ul>
            </CardContent>
          </Card>
        </div>
      )}
    </div>
  );
}
