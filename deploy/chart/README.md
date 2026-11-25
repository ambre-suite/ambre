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

Each image carries a signed provenance attestation in the registry (`SRV-380`). Verify each image
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
  --set domain=ambre.example.org,tls=admin@example.org,existingSecret=ambre
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
* **The web server holds the certificate.** Caddy obtains it from the ACME account `tls`, or
  `tls=internal` gives a certificate of its own authority. The Service keeps the address of each
  client, which the relay limits by.
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
