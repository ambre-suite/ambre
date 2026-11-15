# The dev container

The dev container runs the test stack that a phone on the same network reaches. It holds the
toolchain of `rust-toolchain.toml`, Trunk and the relay. PostgreSQL, Redis and Garage run beside it,
from the images that `docker-compose.yml` pins. The client is served over HTTPS, with the two headers
of `Trunk.toml` that make the page cross-origin isolated. A phone needs both: a browser gives shared
memory, and with it the workers, only to a secure context that is cross-origin isolated.

| File | What it holds |
| :--- | :--- |
| `Dockerfile` | the image: Ubuntu 24.04 as the CI runners run it, rustup, the pinned nightly, Trunk and `just` |
| `compose.yml` | the `dev` service, added to the services of `docker-compose.yml` |
| `devcontainer.json` | the same services, for an editor that opens a dev container |
| `../scripts/dev_stack.py` | the certificates, the relay, the client, and the probe |

The CI workflow `devcontainer.yml` builds the image, starts the stack in it and probes it from the
runner (`ARCH-110`, `ARCH-114`).

## Start the stack

On the host, with Docker and `just`:

```bash
just devcontainer-up            # build the image, start it with its services
just devcontainer-shell         # a shell in the container
```

In the container, with the address of the host on the network of the phone, for example
`192.168.1.20`:

```bash
just phone-stack 192.168.1.20
```

The first start builds the relay and the client cold, which takes 10 to 20 minutes. The client is
then at `https://192.168.1.20:8443`. On the host itself, `https://localhost:8443` serves the same
page. `just phone-stack-probe` checks the page and the relay from the container, then puts one
object through a presigned link and reads it back.

Trunk serves the object store under `/s3/` of the same origin, and forwards it to Garage. The relay
hands out each presigned link there, under `S3_PUBLIC_ENDPOINT`, so a phone puts and gets a blob over
HTTPS. The script sets that variable to `https://<address>:8443/s3`, unless the shell sets it first.

The address must be a private IPv4 address (`10.x`, `172.16.x` to `172.31.x`, `192.168.x`) or a
`.local` name. Find it with `ip -4 addr` on Linux, `ipconfig getifaddr en0` on macOS, or
`ipconfig` on Windows. Port 8443 must be open in the firewall of the host.

## Trust the certificate on the phone

The stack writes a certificate authority once, into `target/dev-stack/ca.pem`, and a server
certificate for the address it is given. The phone trusts the authority once. Each later server
certificate, for a new address, chains to it.

The authority may sign private addresses and `.local` names alone, so its key vouches for no public
site. Its key, `target/dev-stack/ca.key`, still stays on the host: whoever holds it can impersonate
any private address to the phone. Remove the authority from the phone when the trials end.

Copy `ca.pem` to the phone, by a cable, a file share or a mail to yourself. Then:

| Phone | Steps |
| :--- | :--- |
| Android | Settings, Security, Encryption and credentials, Install a certificate, CA certificate, then pick `ca.pem`. Chrome trusts a user authority. The menu names change between vendors |
| iOS, iPadOS | open `ca.pem` in Safari or Files, then Settings, General, VPN and Device Management, install the profile. Then Settings, General, About, Certificate Trust Settings, and turn full trust on for "Ambre dev stack authority" |

Without the authority, a browser shows a warning page for the address. Accepting the warning is
enough to look at the page. It is not enough for a trial: a browser may refuse a worker, a service
worker or shared memory to a page whose certificate it does not trust.

## Check the page on the phone

Open `https://<address>:8443`. A desktop browser reads the console of the phone through remote
debugging: `chrome://inspect` for Chrome on Android, and the Develop menu of Safari on macOS for
iOS. In that console, `crossOriginIsolated` must read `true`.

## Limits

* **The relay runs in development mode.** It takes the credentials of the local stack and accepts
  any origin. Never expose the port beyond a trusted network.
* **On macOS and Windows**, Docker reads the clone through a shared folder. The cold build is then
  slower than on Linux.
