# Cloudflare Tunnel for Parley

Expose a laptop Parley process that is bound to `127.0.0.1` through
`rudeless.ai`, which is already on Cloudflare. The tunnel terminates TLS;
nothing is opened on the conference Wi‑Fi firewall.

| Public hostname | Local origin | Used for |
|---|---|---|
| `https://parley.rudeless.ai` | `http://127.0.0.1:8080` | widget, desk, `/v1`, Telnyx SMS webhook, Vapi tools |
| `wss://parley-uctp.rudeless.ai` | `http://127.0.0.1:7443` | UCTP WebSocket |

Do not route the apex `rudeless.ai` (site + licence APIs) through this tunnel.
SIP/UDP (`127.0.0.1:5060`) is not tunneled.

## One-time Cloudflare setup

On a machine logged into the Cloudflare account that owns `rudeless.ai`:

```sh
cloudflared tunnel login
cloudflared tunnel create parley
cloudflared tunnel route dns parley parley.rudeless.ai
cloudflared tunnel route dns parley parley-uctp.rudeless.ai
```

`route dns` creates proxied CNAMEs. Copy the tunnel UUID into
[`config.yml`](config.yml) (`tunnel` and `credentials-file`).

Alternatively, in Zero Trust → Networks → Tunnels, create a tunnel named
`parley`, add the two public hostnames above, and copy the install token.

## Run on the demo laptop

1. Start Parley on localhost (`8080` + `7443`).
2. Start the tunnel:

```sh
# Named tunnel + config.yml (UUID already filled in)
./scripts/run-cloudflare-tunnel.sh

# Or a dashboard token (no config.yml edit)
CLOUDFLARE_TUNNEL_TOKEN='...' ./scripts/run-cloudflare-tunnel.sh
```

3. Point vendors at the public HTTPS URLs:

- `PARLEY_VAPI_PUBLIC_BASE=https://parley.rudeless.ai`
- Vapi assistant server URL: `https://parley.rudeless.ai/v1/vapi/tools`
- Telnyx inbound SMS: `https://parley.rudeless.ai/v1/sms/inbound`
- Widget: `https://parley.rudeless.ai/widget/?token=…&uctp=wss://parley-uctp.rudeless.ai`
- Desk: `https://parley.rudeless.ai/desk/?uctp=wss://parley-uctp.rudeless.ai`

Keep the hostname reserved. Do not demo on a random `*.trycloudflare.com` URL.
