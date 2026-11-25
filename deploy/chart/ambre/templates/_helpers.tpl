{{/* The name of each object: the release, then the component. */}}
{{- define "ambre.name" -}}
{{- printf "%s-%s" .root.Release.Name .component | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{/* The labels of a component, and the subset a selector reads. */}}
{{- define "ambre.selector" -}}
app.kubernetes.io/name: ambre
app.kubernetes.io/instance: {{ .root.Release.Name }}
app.kubernetes.io/component: {{ .component }}
{{- end -}}

{{- define "ambre.labels" -}}
{{ include "ambre.selector" . }}
app.kubernetes.io/version: {{ .root.Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .root.Release.Service }}
helm.sh/chart: {{ printf "%s-%s" .root.Chart.Name .root.Chart.Version }}
{{- end -}}

{{/* An image of Ambre: by its digest when the values pin one, by its tag otherwise. */}}
{{- define "ambre.image" -}}
{{- $image := index .root.Values.images .component -}}
{{- $registry := .root.Values.images.registry -}}
{{- $repository := ternary (printf "%s/%s" $registry $image.repository) $image.repository (ne $registry "") -}}
{{- if $image.digest -}}
{{- printf "%s@%s" $repository $image.digest -}}
{{- else -}}
{{- printf "%s:%s" $repository (default .root.Chart.AppVersion $image.tag) -}}
{{- end -}}
{{- end -}}

{{/* The Secret of the deployment, which the chart never writes. */}}
{{- define "ambre.secret" -}}
{{- required "existingSecret: run deploy/chart/secret.sh and name its Secret" .Values.existingSecret -}}
{{- end -}}

{{/* A variable read from one key of the Secret of the deployment. */}}
{{- define "ambre.secretEnv" -}}
- name: {{ .name }}
  valueFrom:
    secretKeyRef:
      name: {{ include "ambre.secret" .root }}
      key: {{ .key }}
      {{- if .optional }}
      optional: true
      {{- end }}
{{- end -}}

{{/*
The security context of every container: no root, no capability, no
privilege escalation and a root filesystem that nothing writes.
*/}}
{{- define "ambre.containerSecurity" -}}
securityContext:
  runAsNonRoot: true
  allowPrivilegeEscalation: false
  readOnlyRootFilesystem: true
  capabilities:
    drop: ["ALL"]
{{- end -}}

{{/* The security context of every pod, under the user and group it names. */}}
{{- define "ambre.podSecurity" -}}
automountServiceAccountToken: false
securityContext:
  runAsNonRoot: true
  runAsUser: {{ .uid }}
  runAsGroup: {{ .uid }}
  fsGroup: {{ .uid }}
  seccompProfile:
    type: RuntimeDefault
{{- end -}}

{{/* The S3 endpoint the relay signs for, and the one a presigned link names. */}}
{{- define "ambre.s3Endpoint" -}}
{{- if .Values.garage.enabled -}}
{{- printf "http://%s:3900" (include "ambre.name" (dict "root" . "component" "garage")) -}}
{{- else -}}
{{- required "s3.endpoint: the S3 endpoint of the external object store" .Values.s3.endpoint -}}
{{- end -}}
{{- end -}}

{{- define "ambre.s3PublicEndpoint" -}}
{{- if .Values.garage.enabled -}}
{{- printf "https://%s/s3" .Values.domain -}}
{{- else -}}
{{- required "s3.publicEndpoint: the address a browser reaches the object store at" .Values.s3.publicEndpoint -}}
{{- end -}}
{{- end -}}

{{- define "ambre.domain" -}}
{{- required "domain: the public name of the suite" .Values.domain -}}
{{- end -}}

{{/* A peer of a NetworkPolicy: the pods of one component of this release. */}}
{{- define "ambre.peer" -}}
- podSelector:
    matchLabels: {{- include "ambre.selector" . | nindent 6 }}
{{- end -}}

{{/*
The public addresses of the internet, as NetworkPolicy peers: every address
but the private, shared, link-local and loopback ranges, so a cluster address
and the metadata service of a cloud stay out of reach.
*/}}
{{- define "ambre.publicAddresses" -}}
- ipBlock:
    cidr: 0.0.0.0/0
    except: [10.0.0.0/8, 100.64.0.0/10, 127.0.0.0/8, 169.254.0.0/16, 172.16.0.0/12, 192.168.0.0/16]
- ipBlock:
    cidr: ::/0
    except: ["::1/128", "fc00::/7", "fe80::/10"]
{{- end -}}

{{/* The rule that lets a pod resolve the name of a Service. */}}
{{- define "ambre.dnsEgress" -}}
- to: {{- toYaml .Values.networkPolicy.dns | nindent 4 }}
  ports:
    - {protocol: UDP, port: 53}
    - {protocol: TCP, port: 53}
{{- end -}}

{{/*
The addresses of the nodes, as NetworkPolicy peers (`SRV-410`, `SRV-411`).
The SFU listens on the address of its node, so these blocks bound the egress
of the web server to its signalling. An empty list renders nothing, and so
does a block wider than /8 for IPv4 or /32 for IPv6: no node network of one
cluster spans more, and a wider block reopens the port on the internet.
*/}}
{{- define "ambre.nodeAddresses" -}}
{{- $cidrs := .Values.networkPolicy.nodeCidrs -}}
{{- if not $cidrs -}}
{{- fail "networkPolicy.nodeCidrs: the address blocks of the nodes, where the web server reaches the signalling of the SFU" -}}
{{- end -}}
{{- $peers := list -}}
{{- range $cidrs -}}
{{- $parts := splitList "/" (toString .) -}}
{{- $min := ternary 32 8 (contains ":" (toString .)) -}}
{{- if or (ne (len $parts) 2) (lt (atoi (last $parts)) $min) -}}
{{- fail (printf "networkPolicy.nodeCidrs: %v is no block of /%d or narrower" . $min) -}}
{{- end -}}
{{- $peers = append $peers (dict "ipBlock" (dict "cidr" (toString .))) -}}
{{- end -}}
{{- toYaml $peers -}}
{{- end -}}
