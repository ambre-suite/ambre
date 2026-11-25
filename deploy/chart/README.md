# The chart of Ambre

This directory holds the Helm chart that deploys Ambre on a Kubernetes cluster (`SRV-309`). The
rules it answers live in `specs/1-Foundations/ambre-spec-server.md`, section 3, "A deployment runs
on any host". The configuration of one hosting provider lives in a repository of its own, which
installs this chart at a pinned version.

| File | What it holds |
| :--- | :--- |
| `ambre/` | the chart: the relay, the SFU, the web server, and optional stores |
| `secret.sh` | writes the Secret of a new deployment: a fresh secret for each service |
| `test/` | the kind cluster and the values of the stack under test |

## Install a release

A release publishes the images to `ghcr.io/ambre-suite/ambre` and the chart to
`oci://ghcr.io/ambre-suite/charts`. The chart of a release names each image by its digest
(`SRV-308`), and `ambre-images.txt` on the release lists the same digests.

Each image carries a signed provenance attestation in the registry (`SRV-354`). Verify each image
of a release against the commit of its tag before the install, with `gh` signed in:

```bash
gh release download v<version> -R ambre-suite/ambre -p ambre-images.txt
python3 scripts/image_provenance.py verify ambre-images.txt \
  --source-ref refs/tags/v<version> --source-digest "$(git rev-parse v<version>^{commit})"
```

```bash
kubectl create namespace ambre
./secret.sh ambre | kubectl apply -n ambre -f -
helm install ambre oci://ghcr.io/ambre-suite/charts/ambre --version <version> -n ambre \
  --set domain=ambre.example.org,tls=admin@example.org,existingSecret=ambre \
  --set-json 'networkPolicy.nodeCidrs=["10.0.1.0/24"]'
```

Keep the Secret. Each value in it guards a store, and a new value locks the deployment out of its
own data. The chart writes no Secret and holds no default credential: without `existingSecret`,
it renders nothing.

## What the cluster must give

| Need | Why |
| :--- | :--- |
| a public address for the Service `ambre-web`, ports 80 and 443 | the page, the relay, the object store and the signalling of the SFU, under one certificate |
| a node whose own address is public, and 3481/udp open on it | the media of a call reaches the SFU straight, with no load balancer in front (`CALL-141`) |
| a storage class | the volumes of Postgres, of Garage and of the certificate |

* **The SFU runs on the network of its node.** It binds and announces the address of the node, or
  `sfu.publicAddress`, as the one candidate of its media. `sfu.nodeSelector` pins it to a node with
  a public address. A node behind a 1:1 NAT does not hold its public address, so the SFU cannot
  bind it.
* **The firewall of that node opens 3481/udp, and not 3480/tcp.** The signalling listens on the
  address of the node for the web server alone.
* **`networkPolicy.nodeCidrs` names the address blocks of the nodes** (`SRV-361`). The web server
  reaches 3480/tcp on these blocks alone, since no pod selector names the address of a node. The
  chart renders nothing without a block, or with a block wider than /8 for IPv4 or /32 for IPv6
  (`SRV-362`). Name the subnet of the nodes, not the range of the pods. Cilium matches the address
  of a node against a block only under `policy-cidr-match-mode: nodes`.
* **The web server holds the certificate.** Caddy obtains it from the ACME account `tls`, or
  `tls=internal` gives a certificate of its own authority. The Service keeps the address of each
  client, which the relay limits by.
* **Each pod reaches only the services it names** (`SRV-351`). One NetworkPolicy denies every
  flow of the release, and one per component admits the flows of the table of
  `specs/1-Foundations/ambre-spec-server.md`, section 3. The network plugin of the cluster must
  enforce NetworkPolicy, or nothing is isolated. The SFU runs on the network of its node, outside
  every policy.
* **The policies name the resolver of the cluster.** `networkPolicy.dns` names the CoreDNS of
  `kube-system` by default. A node-local cache needs its own peer there.
* **No pod runs as root.** Every container drops each capability, escalates nothing and writes no
  root filesystem. `just chart-check` holds these rules on every push.

## The stores

Postgres, Redis and Garage run in the cluster by default, from the images of
`deploy/compose/compose.yml`. A deployment with its own stores turns each one off and adds its
address to the Secret:

| Value | Key of the Secret |
| :--- | :--- |
| `postgres.enabled=false` | `database-url` |
| `redis.enabled=false` | `redis-url`, of Redis 7.4 or later (`SRV-241`) |
| `garage.enabled=false`, with `s3.endpoint` and `s3.publicEndpoint` | `s3-access-key-id` and `s3-secret-access-key` keep their names |

A store outside the cluster also needs an egress rule of the relay in `networkPolicy.externalStores`,
or the chart renders nothing. The relay already reaches port 443 of any public address, so an
object store on a public HTTPS endpoint needs none:

```yaml
networkPolicy:
  externalStores:
    - to: [{ipBlock: {cidr: 10.0.4.0/24}}]
      ports: [{port: 5432}, {port: 6379}]
```

A scraper of the metrics of the relay needs its peer in `networkPolicy.metricsFrom` (`SRV-257`).

## The link key of a thread

The relay signs a receipt of each thread link under the seed `thread-link-seed`, and the client
refuses to erase a thread under a receipt its link key verifies (`SYNC-330`). The published web
image pins no key: its client holds an empty link key slot (`ZK-057`). The web pod writes the
`thread-link-key` of the Secret into that slot before Caddy starts, so the published images serve
any install with no build of its own (`SRV-365`):

1. the init container `web-files`, of the web image, copies the client into an `emptyDir`;
2. the init container `link-key`, of the init image, writes the key into that copy;
3. Caddy serves the copy. The web image stays as the release built it.

`secret.sh` draws the seed and its public half, `thread-link-key`, into the Secret. The relay checks
its seed against that key and refuses to start under another key (`SRV-364`). The patch refuses an
empty key, a malformed key and the development key. The pod then stops before Caddy, and the log
says why (`SRV-368`): read it with `kubectl logs <web pod> -c link-key`. No value of the chart
turns the patch off.

**Rotate the link key.** A leaked seed lets anyone sign a receipt, and a receipt over a thread
that does not exist blocks each removal of the channel. A new seed is the only answer:

1. draw a new pair: `openssl genpkey -algorithm ed25519 -outform DER -out link.der`, then the
   seed is `tail -c 32 link.der | od -An -v -tx1 | tr -d ' \n'`, and the key is
   `openssl pkey -inform DER -in link.der -pubout -outform DER | tail -c 32 | od -An -v -tx1 | tr -d ' \n'`;
2. write both into the Secret that `existingSecret` names, here `ambre`: `kubectl -n ambre patch secret ambre --type merge -p
   '{"stringData":{"thread-link-seed":"<seed>","thread-link-key":"<key>"}}'`, then delete
   `link.der`;
3. restart both: `kubectl -n ambre rollout restart deployment/<release>-relay deployment/<release>-web`.
   The relay checks the new seed against the new key (`SRV-364`), and the web pod patches the new
   key into a fresh copy (`SRV-365`);
4. check the served client: `THREAD_LINK_KEY=<key> python3 scripts/link_key.py served https://<domain>`.

The older key then counts nothing, in no client. The relay keeps no receipt: it signs each one
when it answers a registration, so every link it holds gets a receipt under the new key at its next
registration (`SRV-400`). A thread the relay never linked, which a leaked seed could vouch for,
gets none. A browser asks for the Sync worker again before each use (`SRV-401`), so a reload takes
the new key. A tab open across the rotation keeps the older key until it reloads.

**Check the served client.** The patch changes 32 bytes of the Sync worker alone (`SRV-367`).
Download the `ambre-sync-*_bg.wasm` the page names, then run
`python3 scripts/link_key.py zero <file> <file>`, and check it with
`b2sum -l 256 -c ambre-client.manifest` from the release (`ZK-059`). `link_key.py read <file>`
prints the key the served file holds.

## Sign-in

The Secret takes the keys of an identity provider, as `deploy/README.md` describes them:
`oidc-google-client-id` and `oidc-google-client-secret`, or `oidc-issuer`, `oidc-client-id` and
`oidc-client-secret`. A published client image offers Google alone. An issuer of the organization
needs a client built for it, as `deploy/README.md` says, under `images.web`.

## The stack under test

`just chart-test` starts a kind cluster and loads the images that `just selfhost-up` built. It
installs the chart with `test/values.yaml`, then runs the browser tier through port 443 of the
host. That stack signs in by a bare name, which the client build allows only outside a release
(`ZK-045`), and takes a certificate of Caddy's own authority. Neither setting belongs in a
deployment.
