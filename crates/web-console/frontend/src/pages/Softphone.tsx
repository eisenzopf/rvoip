import { useState, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Phone, PhoneOff, PhoneIncoming, MicOff, Mic, Delete } from 'lucide-react';
import { useSipPhone } from '@/hooks/useSipPhone';
import type { SipConfig } from '@/hooks/useSipPhone';

function formatDuration(seconds: number): string {
  const m = Math.floor(seconds / 60).toString().padStart(2, '0');
  const s = (seconds % 60).toString().padStart(2, '0');
  return `${m}:${s}`;
}

const DIALPAD_KEYS = [
  ['1', '2', '3'],
  ['4', '5', '6'],
  ['7', '8', '9'],
  ['*', '0', '#'],
];

export function Softphone() {
  const { t } = useTranslation();
  const phone = useSipPhone();

  const [server, setServer] = useState('ws://127.0.0.1:8080');
  const [domain, setDomain] = useState('call-center.local');
  const [extension, setExtension] = useState('');
  const [password, setPassword] = useState('');
  const [dialInput, setDialInput] = useState('');

  const handleRegister = useCallback(() => {
    const config: SipConfig = { server, domain, extension, password };
    phone.register(config);
  }, [server, domain, extension, password, phone]);

  const handleUnregister = useCallback(() => {
    phone.unregister();
  }, [phone]);

  const handleDial = useCallback(() => {
    if (dialInput.trim()) {
      phone.call(dialInput.trim());
    }
  }, [dialInput, phone]);

  const handleDialpadPress = useCallback((key: string) => {
    setDialInput((prev) => prev + key);
  }, []);

  const handleBackspace = useCallback(() => {
    setDialInput((prev) => prev.slice(0, -1));
  }, []);

  const isRegistered = phone.state === 'registered' || phone.state === 'calling' || phone.state === 'ringing' || phone.state === 'in-call';
  const showDialpad = phone.state === 'registered';
  const showActiveCall = phone.state === 'calling' || phone.state === 'in-call';
  const showIncoming = phone.state === 'ringing';

  const statusDotColor =
    phone.state === 'registered' || phone.state === 'in-call' || phone.state === 'calling' || phone.state === 'ringing'
      ? 'bg-green-500'
      : phone.state === 'error'
        ? 'bg-red-500'
        : phone.state === 'registering'
          ? 'bg-yellow-500 animate-pulse'
          : 'bg-gray-400';

  return (
    <div className="p-6 space-y-6">
      <div>
        <h1 className="text-2xl font-bold tracking-tight">{t('phone.title')}</h1>
        <p className="text-sm text-muted-foreground">{t('phone.subtitle')}</p>
      </div>

      <div className="flex justify-center">
        <Card className="w-[360px]">
          {/* Registration Panel */}
          <CardHeader className="pb-3">
            <CardTitle className="text-sm font-semibold flex items-center gap-2">
              <Phone className="size-4" />
              {t('phone.title')}
              <span className="ml-auto flex items-center gap-1.5 text-xs font-normal text-muted-foreground">
                <span className={`h-2 w-2 rounded-full ${statusDotColor}`} />
                {isRegistered ? t('phone.registered') : t('phone.unregistered')}
              </span>
            </CardTitle>
          </CardHeader>

          <CardContent className="space-y-4">
            {/* Config Fields */}
            {!isRegistered && (
              <div className="space-y-3">
                <div>
                  <label className="text-xs text-muted-foreground">{t('phone.server')}</label>
                  <Input
                    value={server}
                    onChange={(e) => setServer(e.target.value)}
                    placeholder="ws://127.0.0.1:8080"
                    className="mt-1"
                  />
                </div>
                <div>
                  <label className="text-xs text-muted-foreground">{t('phone.domain')}</label>
                  <Input
                    value={domain}
                    onChange={(e) => setDomain(e.target.value)}
                    placeholder="call-center.local"
                    className="mt-1"
                  />
                </div>
                <div>
                  <label className="text-xs text-muted-foreground">{t('phone.extension')}</label>
                  <Input
                    value={extension}
                    onChange={(e) => setExtension(e.target.value)}
                    placeholder="1001"
                    className="mt-1"
                  />
                </div>
                <div>
                  <label className="text-xs text-muted-foreground">{t('phone.password')}</label>
                  <Input
                    type="password"
                    value={password}
                    onChange={(e) => setPassword(e.target.value)}
                    placeholder=""
                    className="mt-1"
                  />
                </div>
              </div>
            )}

            {/* Error message */}
            {phone.error && (
              <div className="rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive">
                {phone.error}
              </div>
            )}

            {/* Register / Unregister */}
            {!isRegistered ? (
              <Button
                className="w-full"
                onClick={handleRegister}
                disabled={phone.state === 'registering' || !extension}
              >
                {phone.state === 'registering' ? '...' : t('phone.register')}
              </Button>
            ) : phone.state === 'registered' ? (
              <Button variant="outline" className="w-full" onClick={handleUnregister}>
                {t('phone.unregister')}
              </Button>
            ) : null}

            {/* Incoming Call Panel */}
            {showIncoming && (
              <div className="space-y-3 rounded-lg border p-4 text-center">
                <PhoneIncoming className="mx-auto size-8 text-green-500 animate-pulse" />
                <p className="text-sm font-medium">
                  {t('phone.incoming')}
                </p>
                <p className="text-lg font-bold">{phone.remoteIdentity}</p>
                <div className="flex gap-2">
                  <Button
                    className="flex-1 bg-green-600 hover:bg-green-700 text-white"
                    onClick={() => phone.answer()}
                  >
                    <Phone className="size-4 mr-1" />
                    {t('phone.answer')}
                  </Button>
                  <Button
                    variant="destructive"
                    className="flex-1"
                    onClick={() => phone.hangup()}
                  >
                    <PhoneOff className="size-4 mr-1" />
                    {t('phone.reject')}
                  </Button>
                </div>
              </div>
            )}

            {/* Active Call Panel */}
            {showActiveCall && (
              <div className="space-y-3 rounded-lg border p-4 text-center">
                <p className="text-xs text-muted-foreground uppercase tracking-wider">
                  {phone.state === 'calling' ? t('phone.call') + '...' : t('phone.duration')}
                </p>
                <p className="text-2xl font-bold">{phone.remoteIdentity}</p>
                {phone.state === 'in-call' && (
                  <p className="text-lg font-mono tabular-nums">
                    {formatDuration(phone.callDuration)}
                  </p>
                )}
                <div className="flex justify-center gap-3">
                  <Button
                    variant={phone.isMuted ? 'secondary' : 'outline'}
                    size="icon"
                    onClick={() => phone.toggleMute()}
                    title={phone.isMuted ? t('phone.unmute') : t('phone.mute')}
                  >
                    {phone.isMuted ? <MicOff className="size-4" /> : <Mic className="size-4" />}
                  </Button>
                  <Button
                    variant="destructive"
                    size="lg"
                    onClick={() => phone.hangup()}
                  >
                    <PhoneOff className="size-4 mr-1" />
                    {t('phone.hangup')}
                  </Button>
                </div>
              </div>
            )}

            {/* Dial Pad */}
            {showDialpad && (
              <div className="space-y-3">
                <div className="flex items-center gap-1">
                  <Input
                    value={dialInput}
                    onChange={(e) => setDialInput(e.target.value)}
                    placeholder={t('phone.target')}
                    className="text-center text-lg font-mono"
                  />
                  <Button variant="ghost" size="icon" onClick={handleBackspace} disabled={!dialInput}>
                    <Delete className="size-4" />
                  </Button>
                </div>

                <div className="grid grid-cols-3 gap-2">
                  {DIALPAD_KEYS.map((row) =>
                    row.map((key) => (
                      <Button
                        key={key}
                        variant="outline"
                        size="lg"
                        className="text-lg font-semibold"
                        onClick={() => handleDialpadPress(key)}
                      >
                        {key}
                      </Button>
                    )),
                  )}
                </div>

                <Button
                  className="w-full bg-green-600 hover:bg-green-700 text-white"
                  onClick={handleDial}
                  disabled={!dialInput.trim()}
                >
                  <Phone className="size-4 mr-1" />
                  {t('phone.call')}
                </Button>
              </div>
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
