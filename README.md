# clusterctl

## Installing

Clone this repo, cd into the root, and run this (--force overwrites previous versions)

```
cargo install --path . --force
```

## Configuration

Create a file at **$HOME/.config/clusterctl/config.toml** with contents like
the following.

```toml
# path to the terraforming repo
terraforming_path = "/home/cmcfarland/Code/terraforming"

# path to the kubernetes-deployments repo
kubernetes_deployments_path = "/home/cmcfarland/Code/kubernetes-deployments"

# path to the kubernetes-deployments ssh key (used to configure argocd)
kubernetes_deployments_ssh_key = "/keybase/team/paperlesspost.infra.keys/ssh/kubernetes-deployments"

# path to the k8s secure_manifests directory; this is different on macos
keybase_secure_manifests_path = "/keybase/team/paperlesspost.kubernetes.secure_manifests"

# TODO: this config value is currently unused
kubernetes_deployments_revision = "master"

# AWS profiles to use when calling terraform or aws cli
infra_profile = "infra_power_user"
v1_profile = "v1_power_user"

# path where kubectl, ssh keys will be downloaded after cluster launch
assets_cache_path = "/home/cmcfarland/.config/clusterctl/assets"
```

Adjust the paths for your machine, and the aws profile names, as well.

## Launching a cluster

Do the following from a single terminal. clusterctl will set env vars for its
subprocesses, but that will leave the rest of your environment untouched. It is
okay to pause during this process and run commands from other terminals, but you
must make sure that you have correct `AWS_PROFILE` and `KUBECONFIG` env vars set

* Log into AWS with `awsmfa` for v1 and infra. No need to set `AWS_PROFILE`. 
* `clusterctl launch-cluster`
* `clusterctl namespace-init`. You have to wait for a bit for the cluster to spin up.
* `clusterctl argo-init`

This takes between 20 to 30 minutes.

## Destroying a cluster

* `clusterctl destroy-cluster`
* `clusterctl destroy-kubernetes-ingress`

This takes between 5 to 10 minutes.

## Completions

The clap cli framework can generate completion scripts. In bash these cannot be
directly `eval`'d, so you must write a script and source it from your shell. Add
something like this to your .bashrc

```bash
if [[ ! -f /tmp/clusterctl_completions.bash ]]; then
    clusterctl completions bash
fi
source /tmp/clusterctl_completions.bash
```

Something similar should work for zsh

