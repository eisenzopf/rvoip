import { useState, useRef, useCallback, useEffect } from 'react';
import { UserAgent, Registerer, Inviter, SessionState, RegistererState } from 'sip.js';
import type { Invitation, Session } from 'sip.js';

export type PhoneState = 'idle' | 'registering' | 'registered' | 'calling' | 'ringing' | 'in-call' | 'error';

export interface SipConfig {
  server: string;
  domain: string;
  extension: string;
  password?: string;
}

export interface SipPhone {
  state: PhoneState;
  error: string | null;
  callDuration: number;
  remoteIdentity: string | null;
  register: (config: SipConfig) => void;
  unregister: () => void;
  call: (target: string) => void;
  answer: () => void;
  hangup: () => void;
  toggleMute: () => void;
  isMuted: boolean;
}

export function useSipPhone(): SipPhone {
  const [state, setState] = useState<PhoneState>('idle');
  const [error, setError] = useState<string | null>(null);
  const [callDuration, setCallDuration] = useState(0);
  const [remoteIdentity, setRemoteIdentity] = useState<string | null>(null);
  const [isMuted, setIsMuted] = useState(false);

  const uaRef = useRef<UserAgent | null>(null);
  const registererRef = useRef<Registerer | null>(null);
  const sessionRef = useRef<Session | null>(null);
  const invitationRef = useRef<Invitation | null>(null);
  const configRef = useRef<SipConfig | null>(null);
  const durationTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const clearDurationTimer = useCallback(() => {
    if (durationTimerRef.current) {
      clearInterval(durationTimerRef.current);
      durationTimerRef.current = null;
    }
  }, []);

  const startDurationTimer = useCallback(() => {
    clearDurationTimer();
    setCallDuration(0);
    const start = Date.now();
    durationTimerRef.current = setInterval(() => {
      setCallDuration(Math.floor((Date.now() - start) / 1000));
    }, 1000);
  }, [clearDurationTimer]);

  const setupSessionListeners = useCallback((session: Session) => {
    session.stateChange.addListener((newState: SessionState) => {
      if (newState === SessionState.Established) {
        setState('in-call');
        startDurationTimer();
      }
      if (newState === SessionState.Terminated) {
        clearDurationTimer();
        setCallDuration(0);
        setRemoteIdentity(null);
        setIsMuted(false);
        sessionRef.current = null;
        invitationRef.current = null;
        setState(registererRef.current ? 'registered' : 'idle');
      }
    });
  }, [startDurationTimer, clearDurationTimer]);

  const register = useCallback(async (config: SipConfig) => {
    try {
      setState('registering');
      setError(null);
      configRef.current = config;

      const uri = UserAgent.makeURI(`sip:${config.extension}@${config.domain}`);
      if (!uri) {
        setError('Invalid SIP URI');
        setState('error');
        return;
      }

      const ua = new UserAgent({
        uri,
        transportOptions: { server: config.server },
        authorizationUsername: config.extension,
        authorizationPassword: config.password || '',
        delegate: {
          onInvite: (invitation: Invitation) => {
            invitationRef.current = invitation;
            sessionRef.current = invitation;
            setRemoteIdentity(invitation.remoteIdentity.uri.user || 'Unknown');
            setState('ringing');
            setupSessionListeners(invitation);
          },
        },
      });

      await ua.start();
      uaRef.current = ua;

      const registerer = new Registerer(ua);
      registererRef.current = registerer;

      registerer.stateChange.addListener((newState: RegistererState) => {
        if (newState === RegistererState.Registered) {
          setState('registered');
        }
        if (newState === RegistererState.Unregistered) {
          setState('idle');
        }
      });

      await registerer.register();
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : 'Registration failed';
      setError(message);
      setState('error');
    }
  }, [setupSessionListeners]);

  const unregister = useCallback(async () => {
    try {
      if (sessionRef.current && sessionRef.current.state !== SessionState.Terminated) {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const sess = sessionRef.current as any;
        if (typeof sess.bye === 'function') {
          sess.bye();
        }
      }
      clearDurationTimer();

      if (registererRef.current) {
        await registererRef.current.unregister();
        registererRef.current = null;
      }
      if (uaRef.current) {
        await uaRef.current.stop();
        uaRef.current = null;
      }
      sessionRef.current = null;
      invitationRef.current = null;
      configRef.current = null;
      setRemoteIdentity(null);
      setCallDuration(0);
      setIsMuted(false);
      setState('idle');
      setError(null);
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : 'Unregister failed';
      setError(message);
    }
  }, [clearDurationTimer]);

  const call = useCallback(async (target: string) => {
    const ua = uaRef.current;
    const config = configRef.current;
    if (!ua || !config) return;

    try {
      const targetUri = UserAgent.makeURI(`sip:${target}@${config.domain}`);
      if (!targetUri) {
        setError('Invalid target URI');
        return;
      }

      const inviter = new Inviter(ua, targetUri);
      sessionRef.current = inviter;
      setRemoteIdentity(target);
      setState('calling');
      setupSessionListeners(inviter);
      await inviter.invite();
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : 'Call failed';
      setError(message);
      setState('registered');
    }
  }, [setupSessionListeners]);

  const answer = useCallback(async () => {
    const invitation = invitationRef.current;
    if (!invitation) return;

    try {
      await invitation.accept();
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : 'Answer failed';
      setError(message);
    }
  }, []);

  const hangup = useCallback(async () => {
    const session = sessionRef.current;
    if (!session) return;

    try {
      if (session.state === SessionState.Established) {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const sess = session as any;
        if (typeof sess.bye === 'function') {
          sess.bye();
        }
      } else if (session.state === SessionState.Establishing) {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const sess = session as any;
        if (typeof sess.cancel === 'function') {
          sess.cancel();
        }
      }

      // For incoming ringing calls, reject
      const invitation = invitationRef.current;
      if (invitation && state === 'ringing') {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const inv = invitation as any;
        if (typeof inv.reject === 'function') {
          inv.reject();
        }
      }
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : 'Hangup failed';
      setError(message);
    }
  }, [state]);

  const toggleMute = useCallback(() => {
    const session = sessionRef.current;
    if (!session || session.state !== SessionState.Established) return;

    try {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const sdh = (session as any).sessionDescriptionHandler;
      if (sdh && sdh.peerConnection) {
        const pc = sdh.peerConnection as RTCPeerConnection;
        pc.getSenders().forEach((sender: RTCRtpSender) => {
          if (sender.track && sender.track.kind === 'audio') {
            sender.track.enabled = isMuted;
          }
        });
        setIsMuted(!isMuted);
      }
    } catch (_err: unknown) {
      // Mute toggle is best-effort
    }
  }, [isMuted]);

  useEffect(() => {
    return () => {
      clearDurationTimer();
    };
  }, [clearDurationTimer]);

  return {
    state,
    error,
    callDuration,
    remoteIdentity,
    register,
    unregister,
    call,
    answer,
    hangup,
    toggleMute,
    isMuted,
  };
}
