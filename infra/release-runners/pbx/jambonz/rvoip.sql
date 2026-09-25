-- Release-lab-only identities. These are deterministic fixtures, not secrets.
UPDATE webhooks
SET url = 'http://auth-server:4000/auth',
    username = 'fixture-user',
    password = 'fixture-pass'
WHERE webhook_sid = '90dda62e-0ea2-47d1-8164-5bd49003476c';

UPDATE accounts
SET sip_realm = 'sip.rvoip.test'
WHERE account_sid = 'ed649e33-e771-403a-8c99-1780eabbc803';

UPDATE service_providers
SET root_domain = 'sip.rvoip.test'
WHERE service_provider_sid = '3f35518f-5a0d-4c2e-90a5-2407bb3b36f0';
