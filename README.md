# clusterctl

## Launching a cluster

Do the following from a single terminal. clusterctl will set env vars for its
subprocesses, but that will leave the rest of your environment untouched. It is
okay to pause during this process and run commands from other terminals, but you
must make sure that you have correct `AWS_PROFILE` and `KUBECONFIG` env vars set

* Log into AWS with `awsmfa` for v1 and infra. No need to set `AWS_PROFILE`. 
* `clusterctl launch-cluster`
* `clusterctl namespace-init`
* `clusterctl argo-init`

