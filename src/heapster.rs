pub fn heapster_app_template(ns: &str, cluster_id: &str) -> String {
    // ns, ns, cluster_id
    format!(
        r#"
---
apiVersion: argoproj.io/v1alpha1
kind: Application
metadata:
  name: pp-heapster-{}
spec:
  destination:
    namespace: {}
    server: https://kubernetes.default.svc
  ignoreDifferences:
    - group: extensions
      kind: Deployment
      jsonPointers:
      - /spec/template/spec/containers/0/resources
  project: default
  source:
    helm:
      valueFiles:
      - values.yaml
      - values-{}.yaml
    path: charts/pp-heapster
    repoURL: git@github.com:paperlesspost/kubernetes-deployments
    targetRevision: HEAD
  syncPolicy:
    automated: {{}}

"#,
        ns, ns, cluster_id
    )
}
