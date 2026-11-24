{{- define "hypolia.fullname" -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "hypolia.labels" -}}
app.kubernetes.io/name: hypolia
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{- define "hypolia.image" -}}
{{- $tag := .tag | default .global.imageTag -}}
{{- if and .global.immutableTags (or (not $tag) (eq $tag "latest")) -}}
{{- fail "global.immutableTags: imageTag doit être un sha ou une version, jamais vide ni latest" -}}
{{- end -}}
{{- printf "%s/%s:%s" .global.imageRegistry .repository $tag -}}
{{- end -}}

{{- define "hypolia.forwardingSecretName" -}}
{{- if .Values.secrets.existingForwardingSecret -}}
{{- .Values.secrets.existingForwardingSecret -}}
{{- else -}}
{{- printf "%s-forwarding" (include "hypolia.fullname" .) -}}
{{- end -}}
{{- end -}}

{{- define "hypolia.orchestratorTokenSecretName" -}}
{{- if .Values.secrets.existingOrchestratorTokenSecret -}}
{{- .Values.secrets.existingOrchestratorTokenSecret -}}
{{- else if .Values.secrets.devOrchestratorToken -}}
{{- printf "%s-orch-token" (include "hypolia.fullname" .) -}}
{{- end -}}
{{- end -}}

{{- define "hypolia.s3SecretName" -}}
{{- if .Values.s3.existingSecret -}}
{{- .Values.s3.existingSecret -}}
{{- else if .Values.s3.enabled -}}
{{- printf "%s-s3" (include "hypolia.fullname" .) -}}
{{- end -}}
{{- end -}}
