#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]
use anyhow::{anyhow, Error};
use clap::{App, Arg, Shell, SubCommand};
use console::Style;
use dialoguer::{theme::ColorfulTheme, Confirmation, Editor, Input, Select};
use std::env;
use std::env::args;
use std::fs::File;
use std::io;
use std::io::prelude::*;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::str::FromStr;

mod config;
mod heapster;
mod helm;
mod kubectl;
mod runner;
mod terraform;

use config::Config;
use runner::{Cmd, Expect};

fn main() -> Result<(), Error> {
    let default_dir = home_with(".config/clusterctl");
    let default_config = home_with(".config/clusterctl/config.toml");
    create_dir(&default_dir.clone()).expect("could not create default config dir");

    let mut app = App::new("clusterctl")
        .about("Interactive wrapper that stands up and tears down Kubernetes")
        .version("0.1.0")
        .arg(
            Arg::with_name("config")
                .short("c")
                .long("config")
                .help("path to config.toml")
                .takes_value(true)
                .default_value(&default_config),
        )
        .subcommands(vec![
            SubCommand::with_name("cache-assets").about("cache generated kube configs locally"),
            SubCommand::with_name("completions")
                .about("generate a completions script for your shell")
                .arg(
                    Arg::with_name("shell")
                        .possible_values(&["bash", "zsh"])
                        .required(true),
                ),
            SubCommand::with_name("destroy-cluster").about("destroy a k8s cluster"),
            SubCommand::with_name("destroy-kubernetes-ingress")
                .about("destroy the ingress DNS records"),
            SubCommand::with_name("launch-cluster")
                .about("launch a new k8s cluster with the terraform tectonic installer"),
            SubCommand::with_name("namespace-init")
                .about("create namespaces with secrets and config maps"),
            SubCommand::with_name("argo-init").about("install and configure argo on a cluster"),
            SubCommand::with_name("tool-check").about("check for required tools on PATH"),
        ]);

    let (bash, zsh) = completions(&mut app);

    // Load config
    let matches = app.get_matches();
    let config_path = matches
        .value_of("config")
        .ok_or(anyhow!("could not locate config"))?;
    let config = Config::from_file(&config_path)?;

    // Subcommands
    match matches.subcommand() {
        ("completions", Some(args)) => match args.value_of("shell").unwrap() {
            "bash" => io::stdout().lock().write_all(&bash).unwrap(),
            "zsh" => io::stdout().lock().write_all(&bash).unwrap(),
            _ => unreachable!(),
        },
        ("destroy-cluster", _) => destroy_cluster(&config)?,
        ("destroy-kubernetes-ingress", _) => destroy_kubernetes_ingress(&config, None)?,
        ("launch-cluster", _) => launch_cluster(&config)?,
        ("namespace-init", _) => namespace_init(&config, None)?,
        ("argo-init", _) => argo_init(&config, None)?,
        _ => return Err(anyhow!("you must provide a subcommand")),
    }

    Ok(())
}

fn completions(app: &mut App) -> (Vec<u8>, Vec<u8>) {
    let mut bash = Vec::<u8>::new();
    let mut zsh = Vec::<u8>::new();
    let bin = "clusterctl";
    app.gen_completions_to(bin, Shell::Bash, &mut bash);
    app.gen_completions_to(bin, Shell::Zsh, &mut zsh);
    (bash, zsh)
}

fn argo_init(conf: &Config, cluster_id: Option<String>) -> Result<(), Error> {
    let cluster_id = cluster_id.unwrap_or(pick_cluster_id_prompt()?);
    let infra_profile = &conf.infra_profile;

    // fetch kubeconfig
    let bucket = assets_bucket_name(&cluster_id, infra_profile)?;
    let cache_dir = Path::new(&conf.assets_cache_path).join(&cluster_id);
    create_dir(&cache_dir)?;
    let kubeconfig_path = cache_dir.join("kubeconfig");
    let path = kubeconfig_path
        .to_str()
        .ok_or(anyhow!("malformed assets path"))?;
    download_kubeconfig(&bucket, infra_profile, path)?;

    env::remove_var("KUBECONFIG");
    env::set_var("KUBECONFIG", &path);

    // create namespace
    let c = Cmd::new(vec!["kubectl", "create", "ns", "argocd"]);
    prompt_run! { "Execute?", c, Expect::Success };

    let path = Path::new(&conf.kubernetes_deployments_path).to_path_buf();

    let mut c = Cmd::new(vec!["helm", "dep", "update", "charts/pp-argo-cd"]);
    let c = c.dir(path.clone());
    prompt_run! { "Execute?", c, Expect::Success };

    // Template your ArgoCD YAML manifests
    let d_ns = default_namespace(&cluster_id);
    let chart = format!("charts/pp-argo-cd/values-{}.yaml", d_ns);
    let mut c = Cmd::new(vec![
        "helm",
        "template",
        "-n",
        "argocd",
        "-f",
        &chart,
        "charts/pp-argo-cd",
    ]);
    let c = c.dir(path.clone());
    let outfile = PathBuf::new().join("/tmp/argo_template.yaml");
    let c = c.writes_file(outfile);
    prompt_run! { "Template pp-argo-cd chart? File will be written to /tmp/argo_template.yaml", c, Expect::Success };
    println!("Note: the warning \"destination for dexConfig is a table\" can be ignored");

    // Deploy ArgoCD
    let c = Cmd::new(vec![
        "kubectl",
        "apply",
        "-n",
        "argocd",
        "-f",
        "/tmp/argo_template.yaml",
    ]);
    prompt_run! { "Deploy argocd?", c, Expect::Success };

    // TODO pause here for a couple of minutes while argo deploys
    pause("Wait for a couple of minutes while the ELB comes up");

    let argocd_server = kubectl::get_argo_server_name()?;
    println!("\nDiscovered argocd-server pod: {}", &argocd_server);
    let argo_elb = kubectl::get_argo_elb()?;
    println!("\nDiscovered argocd-server elb: {}", &argo_elb);

    println!("\nSkipping creation of DNS records for argocd or argocd-beta subdomain");
    println!("https://github.com/paperlesspost/terraforming/pull/891");

    // # Login to ArgoCD
    let c = Cmd::new(vec![
        "argocd",
        "login",
        &argo_elb,
        "--username",
        "admin",
        "--password",
        &argocd_server,
    ]);
    prompt_run!("Log in to argo?", c, Expect::Success);

    //  Add our kubernetes-deployments repo to Argo
    let repo = "git@github.com:paperlesspost/kubernetes-deployments";
    let pk = &conf.kubernetes_deployments_ssh_key;
    let c = Cmd::new(vec![
        "argocd",
        "repo",
        "add",
        repo,
        "--ssh-private-key-path",
        pk,
    ]);
    prompt_run!("Add git repo and private key?", c, Expect::Success);

    // Patch argocd-secret
    println!("\nThe argocd-secret must be patched with a value from 1Password");
    println!("We will open a buffer in you editor and you will write this secret");
    println!("to the first line. Do not write more than one line.");
    if continue_prompt("Open buffer in your $EDITOR to input the secret?") {
        if let Some(dex_secret) = Editor::new()
            .edit("Enter 1P entry 'ArgoCD Beta Github App' (or equivalent) on exactly one line")
            .unwrap()
        {
            let trimmed = dex_secret.trim();
            // Brackets {} are annoying to escape, so we build each part: left, secret, right
            let (left, right) = ("{ \"data\": { \"dex.github.clientSecret\": \"", "\"}}");
            let patch = format!("{}{}{}", left, &trimmed, right);
            let c = Cmd::new(vec![
                "kubectl",
                "patch",
                "secret",
                "argocd-secret",
                "-n",
                "argocd",
                "--patch",
                &patch,
            ]);
            prompt_run!("Patch argocd-secret?", c, Expect::Success);
        } else {
            println!("You must enter a dex secret. Exiting.");
            std::process::exit(1);
        }
    }

    // Create a bootstrap project
    let c = Cmd::new(vec![
        "argocd",
        "proj",
        "create",
        "bootstrap",
        "-d",
        "*,*",
        "-s",
        "*",
    ]);
    prompt_run!("Create argocd bootstrap project?", c, Expect::Success);

    pause("Wait a few seconds and let the bootstrap project initialize");

    // # Allow bootstrap project to manage any k8s resource GROUP and KIND
    // argocd proj allow-cluster-resource bootstrap "*" "*"
    let c = Cmd::new(vec![
        "argocd",
        "proj",
        "allow-cluster-resource",
        "bootstrap",
        "*",
        "*",
    ]);
    prompt_run!(
        "Let bootstrap project manage any k8s resource?",
        c,
        Expect::Success
    );

    // Launch platform services
    let cluster_app_manifest = format!("bootstrap/{}/cluster.yaml", d_ns);
    let mut c = Cmd::new(vec!["argocd", "app", "create", "-f", &cluster_app_manifest]);
    let c = c.dir(path.clone());
    prompt_run!(
        "Create bootstrap Application CRD for cluster services (this will launch a bunch of pods)?",
        c,
        Expect::Success
    );

    pause("Wait for a minute for chartmuseum to come online");

    // Patch argocd-cm config map
    let left = "{ \"data\": { \"helm.repositories\": \"- name: paperless\\n  type: helm\\n  url: http://chartmuseum.";
    let right = "\\n\"}}";
    let patch = format!("{}{}{}", left, d_ns, right);
    let c = Cmd::new(vec![
        "kubectl",
        "patch",
        "configmap",
        "argocd-cm",
        "-n",
        "argocd",
        "--patch",
        &patch,
    ]);
    prompt_run!(
        "Patch argocd-cm configmap with our cluster's chartmuseum url?",
        c,
        Expect::Success
    );

    // Deploy heapster
    let heapster_path = "/tmp/pp-heapster.yaml";
    let mut f = std::fs::File::create(&heapster_path)?;
    let tmpl = heapster::heapster_app_template(&d_ns, &cluster_id);
    f.write_all(&tmpl.into_bytes())?;
    println!("\nAn Application CRD template has been written to /tmp/pp-heapster.yaml");
    let c = Cmd::new(vec!["argocd", "app", "create", "-f", &heapster_path]);
    prompt_run!("Deploy heapster?", c, Expect::Success);

    // Deploy paperless services
    println!("\n We are ready to deploy paperless services");
    let pp_svcs_manifest = format!("bootstrap/{}/paperless-services.yaml", d_ns);
    let mut c = Cmd::new(vec!["argocd", "app", "create", "-f", &pp_svcs_manifest]);
    let c = c.dir(path.clone());
    prompt_run!(
        "Deploy pp services (this will launch all our apps)?",
        c,
        Expect::Success
    );

    println!("\nAll services deployed.");

    Ok(())
}

fn namespace_init(conf: &Config, cluster_id: Option<String>) -> Result<(), Error> {
    let cluster_id = cluster_id.unwrap_or(pick_cluster_id_prompt()?);
    let infra_profile = &conf.infra_profile;

    // fetch kubeconfig
    let bucket = assets_bucket_name(&cluster_id, infra_profile)?;
    let cache_dir = Path::new(&conf.assets_cache_path).join(&cluster_id);
    create_dir(&cache_dir)?;
    let kubeconfig_path = cache_dir.join("kubeconfig");
    let path = kubeconfig_path
        .to_str()
        .ok_or(anyhow!("malformed assets path"))?;
    download_kubeconfig(&bucket, infra_profile, path)?;

    // Child processes will inherit our custom KUBECONFIG
    env::remove_var("KUBECONFIG");
    env::set_var("KUBECONFIG", &path);

    // create namespace
    let d_ns = default_namespace(&cluster_id);
    let c = Cmd::new(vec!["kubectl", "create", "ns", d_ns]);
    prompt_run! { "Execute?", c, Expect::Success };

    let secure_manifests = &conf.keybase_secure_manifests_path;
    let shared_secrets = Path::new(secure_manifests).join("secrets/shared");
    let default_ns_secrets = Path::new(secure_manifests).join(format!("secrets/{}", d_ns));
    let default_ns_configmaps = Path::new(secure_manifests).join(format!("configMaps/{}", d_ns));

    let c = Cmd::new(vec![
        "kubectl",
        "create",
        "-n",
        "kube-system",
        "-Rf",
        shared_secrets.to_str().unwrap(),
    ]);
    prompt_run! { "Deploy shared secrets?", c, Expect::Success }

    let c = Cmd::new(vec![
        "kubectl",
        "create",
        "-n",
        d_ns,
        "-Rf",
        default_ns_secrets.to_str().unwrap(),
    ]);
    prompt_run! { "Deploy default namespace secrets? NOTE: An error is expected", c, Expect::Failure }

    let c = Cmd::new(vec![
        "kubectl",
        "create",
        "-n",
        d_ns,
        "-Rf",
        default_ns_configmaps.to_str().unwrap(),
    ]);
    prompt_run! { "Deploy default namespace config maps?", c, Expect::Success }

    let from_literal = format!("--from-literal=cluster-name={}", &cluster_id);
    let c = Cmd::new(vec![
        "kubectl",
        "create",
        "configmap",
        "cluster-info",
        &from_literal,
        "-n",
        "kube-system",
    ]);
    prompt_run! { "Create cluster-info config map in kube-system namespace?", c, Expect::Success }

    let c = Cmd::new(vec![
        "kubectl",
        "create",
        "configmap",
        "cluster-info",
        &from_literal,
        "-n",
        &d_ns,
    ]);
    prompt_run! { "Create cluster-info config map in default namespace?", c, Expect::Success }

    Ok(())
}

fn launch_cluster(conf: &Config) -> Result<(), Error> {
    println!(
        r#"
This will step you through launching a cluster.
1. You will be prompted before each step
2. You will be shown the commands that will be run
3. STDOUT and STDERR will be printed to your console, as if you'd run the commands manually.
"#
    );
    let infra_profile = &conf.infra_profile;
    let cluster_id = pick_cluster_id_prompt()?;
    let path = Path::new(&conf.terraforming_path.clone()).join("projects/kubernetes-tectonic");

    let mut c = Cmd::new(vec!["terraform", "get", "-update"]);
    let c = c.dir(path.clone());
    prompt_run! {"Execute command?", c, Expect::Success};

    println!("\nSelect the correct workspace");
    let mut c = Cmd::new(vec!["terraform", "workspace", "select", &cluster_id]);
    let c = c.dir(path.clone());
    prompt_run! {"Execute command?", c, Expect::Success};

    println!("\nPlan changes to kubernetes-tectonic");
    let tfvars = format!("{}.tfvars", &cluster_id);
    let mut c = Cmd::new(vec![
        "terraform",
        "plan",
        "-out",
        "tfplan.out",
        "-var-file",
        &tfvars,
    ]);
    let c = c.env("AWS_PROFILE", &infra_profile);
    let c = c.dir(path.clone());
    prompt_run! {"Execute command?", c, Expect::Success};

    println!("\nApply kubernetes-tectonic");
    let mut c = Cmd::new(vec!["terraform", "apply", "tfplan.out"]);
    let c = c.env("AWS_PROFILE", &infra_profile);
    let c = c.dir(path.clone());
    prompt_run! {"Execute command?", c, Expect::Failure};

    println!("\nRe-plan changes to kubernetes-tectonic after expected error");
    let mut c = Cmd::new(vec![
        "terraform",
        "plan",
        "-out",
        "tfplan.out",
        "-var-file",
        &tfvars,
    ]);
    let c = c.env("AWS_PROFILE", &infra_profile);
    let c = c.dir(path.clone());
    prompt_run! {"Execute command?", c, Expect::Success};

    println!("\nRe-apply kubernetes-tectonic");
    let mut c = Cmd::new(vec!["terraform", "apply", "tfplan.out"]);
    let c = c.env("AWS_PROFILE", &infra_profile);
    let c = c.dir(path.clone());
    prompt_run! {"Execute command?", c, Expect::Success};

    println!("\nEnjoy your new cluster :)");

    Ok(())
}

fn destroy_kubernetes_ingress(conf: &Config, cluster_id: Option<String>) -> Result<(), Error> {
    let cluster_id = cluster_id.unwrap_or(pick_cluster_id_prompt()?);
    let path = Path::new(&conf.terraforming_path.clone()).join("projects/kubernetes-ingress");
    let v1_profile = &conf.v1_profile;

    println!("\nWe will now step through destroying the kubernetes-ingress project");
    println!("First, we must select the right workspace");
    println!("Path: {:?}", path);
    println!("Command: terraform workspace select {}", cluster_id);
    if continue_prompt("Execute command?") {
        let status = terraform::workspace_select(&path, &cluster_id, &v1_profile)?;
        if !status.success() {
            return Err(anyhow!("could not select workspace"));
        }
    } else {
        return Ok(());
    }

    println!(
        "\nWe will now prepare a -destroy plan against terraforming/projects/kubernetes-ingress"
    );
    println!("Path: {:?}", path);
    println!(
        "Command: terraform plan -out tfplan.out -var-file {}.tfvars -destroy -detailed-exitcode",
        cluster_id
    );
    if continue_prompt("Execute command?") {
        let status = terraform::plan_destroy_with_tfvars_file(&path, &cluster_id, v1_profile)?;
        match status.code() {
            Some(1) => return Err(anyhow!("unexpected error in -destroy plan")),
            _ => { /* no op - continue */ }
        }
    } else {
        return Ok(());
    }

    println!(
        "We are ready to apply. This will DESTROY DNS routes that point to {}",
        cluster_id
    );
    println!("Path: {:?}", path);
    println!("Command: terraform apply tfplan.out");
    if continue_prompt("Execute command?") {
        let status = terraform::apply(&path, v1_profile)?;
        if !status.success() {
            return Err(anyhow!("unexpected error"));
        }
    } else {
        return Ok(());
    }

    println!("\nWe have removed the DNS records!");

    let url = cluster_elbs_url(&cluster_id);
    println!(
        r#"
A manual step is required in the AWS web console.
There will be 3 ELBs created by Kubernetes that will be left running.

The following URL will show all ELBs related to cluster id {}

{}

INSPECT EACH ONE CAREFULLY BEFORE YOU DELETE IT. There should be 0 live instances
associated with the ELBs you delete.
"#,
        cluster_id, url
    );

    if continue_prompt("Open this url in your browser? (remember to use the the infra profile)") {
        open_browser(&url)?;
    }

    Ok(())
}

fn destroy_cluster(conf: &Config) -> Result<(), Error> {
    let theme = prompt_theme();

    println!(
        r#"
This will step you through destroying a cluster.
1. You will be prompted before each step
2. You will be shown the commands that will be run
3. STDOUT and STDERR will be printed to your console, as if you'd run the commands manually.
"#
    );
    if !continue_prompt("Do you want to proceed? (Use arrows)") {
        return Ok(());
    }
    println!("");

    let infra_profile = &conf.infra_profile;
    let cluster_id = pick_cluster_id_prompt()?;

    // destroy kubernetes-alarms
    // TODO

    // destroy kubernetes-tectonic
    let path = Path::new(&conf.terraforming_path.clone()).join("projects/kubernetes-tectonic");
    println!(
        "\nWe will now prepare a -destroy plan against terraforming/projects/kubernetes-tectonic"
    );
    println!("First, we must select the right workspace");
    println!("Path: {:?}", path);
    println!("Command: terraform workspace select {}", cluster_id);
    if !continue_prompt("Execute command?") {
        return Ok(());
    }

    let status = terraform::workspace_select(&path, &cluster_id, infra_profile)?;
    if !status.success() {
        return Err(anyhow!("terraform workspace select"));
    }

    println!("\nNext, we can optionally remove state that sometimes causes problems");
    println!("Path: {:?}", path);
    println!(
        r#"Command: terraform state rm \ 
    module.tectonic-aws.module.bootkube.template_dir.bootkube \
    module.tectonic-aws.module.tectonic.template_dir.tectonic \
    module.tectonic-aws.module.bootkube.template_dir.bootkube_bootstrap"#
    );

    let idx = Select::with_theme(&theme)
        .with_prompt("Execute command or skip?")
        .items(&["execute", "skip"])
        .interact()?;

    if idx == 0 {
        let status = terraform::state_rm(
            &path,
            &[
                "module.tectonic-aws.module.bootkube.template_dir.bootkube",
                "module.tectonic-aws.module.tectonic.template_dir.tectonic",
                "module.tectonic-aws.module.bootkube.template_dir.bootkube_bootstrap",
            ],
            infra_profile,
        )?;
        if !status.success() {
            return Err(anyhow!("error: terraform state rm"));
        }
    }

    println!("\nNext, we actually plan");
    println!("Path: {:?}", path);
    println!(
        "Command: terraform plan -out tfplan.out -var-file {}.tfvars -destroy -detailed-exitcode",
        cluster_id
    );
    if continue_prompt("Execute command?") {
        let status = terraform::plan_destroy_with_tfvars_file(&path, &cluster_id, infra_profile)?;
        // NOTE: we should be able to match on exit code 0 here to indicate no
        // diff was found, but it does not seem to work. We get exit code 2,
        // even when the plan shows no diff (e.g. -destroy against a cluster
        // that doesn't exist).
        match status.code() {
            Some(1) => {
                return Err(anyhow!(
                    r#"Could not "terraform plan". 
You probably need to re-run this tool and remove problematic bootkube/tectonic state."#
                ));
            }
            _ => { /* no op - continue */ }
        }
    } else {
        return Ok(());
    }

    println!("The plan we just ran should show approximately 120 resources to delete.");
    println!(
        r#"Unfortunately, the output of the plan does not contain the cluster id
so we must double check the workspace we are on!
"#
    );
    println!("Path: {:?}", path);
    println!("Command: terraform workspace show");
    if continue_prompt("Execute_command?") {
        println!("");
        let status = terraform::workspace_show(&path, infra_profile)?;
        if !status.success() {
            return Err(anyhow!("terraform workspace show"));
        }
    } else {
        return Ok(());
    }

    if !continue_prompt("\nAre we on the right workspace?") {
        return Err(anyhow!("wrong workspace"));
    }

    // destroy kubernetes-ingress
    println!("\nWe are ready to destroy the cluster. THERE IS NO GOING BACK");
    println!("Path: {:?}", path);
    println!("Command: terraform apply tfplan.out");
    if continue_prompt("Execute command?") {
        let status = terraform::apply(&path, infra_profile)?;
        if !status.success() {
            println!("\nterraform apply encountered an error, but this is expected.");
        }
    } else {
        return Ok(());
    }

    // TODO do we need TWO re-plan and re-applies?

    println!("We will now create another -destroy plan to ensure all resources are cleaned up");
    println!("This plan should show no diff");
    println!("Path: {:?}", path);
    println!(
        "Command: terraform plan -out tfplan.out -var-file {}.tfvars -destroy -detailed-exitcode",
        cluster_id
    );
    if continue_prompt("Execute command?") {
        let status = terraform::plan_destroy_with_tfvars_file(&path, &cluster_id, infra_profile)?;
        match status.code() {
            Some(1) => return Err(anyhow!("unexpected error")),
            _ => { /* no op - continue */ }
        }
    } else {
        return Ok(());
    }

    println!(
        "\nCluster destroy complete. DNS and ELBs associated with {} may still be up",
        cluster_id
    );
    if continue_prompt("Do you want to move on to destroying DNS and ELBs?") {
        destroy_kubernetes_ingress(conf, Some(cluster_id.to_owned()))?;
    }

    Ok(())
}

/// Join a path to the HOME directory. Panics on any error. HOME env var must be set.
fn home_with(path: &'static str) -> String {
    Path::new(&env::var("HOME").expect("HOME env var unset"))
        .join(path)
        .to_str()
        .unwrap()
        .to_owned()
}

/// Create a directory if it does not exist.
fn create_dir<P: AsRef<Path>>(p: P) -> Result<(), Error> {
    if !p.as_ref().exists() {
        std::fs::create_dir_all(p.as_ref())?;
    }
    Ok(())
}

fn valid_clusters() -> Vec<&'static str> {
    vec![
        "development0",
        "development1",
        "development2",
        "production0",
        "production1",
        "production2",
    ]
}

fn assets_bucket_name(cluster_id: &str, profile: &str) -> Result<String, Error> {
    let matcher = format!("a{}", cluster_id);
    use std::str::FromStr;
    let mut cmd = Command::new("aws");
    cmd.env("AWS_PROFILE", profile);
    cmd.args(vec![
        "s3api",
        "list-buckets",
        "--query",
        "Buckets[].Name",
        "--output",
        "text",
    ]);
    let output = cmd.output()?;
    if !output.status.success() {
        // println!("STDOUT {:?}", std::str::from_utf8(&output.stderr)?);
        // println!("STDERR {:?}", std::str::from_utf8(&output.stdout)?);
        return Err(anyhow!("listing buckets with aws cli failed"));
    }
    let s = std::str::from_utf8(&output.stdout)?;
    for item in s.split_whitespace() {
        if item.starts_with(&matcher) {
            return Ok(String::from_str(item)?);
        }
    }
    Err(anyhow!("could not locate assets bucket for {}", cluster_id))
}

fn download_kubeconfig(bucket: &str, profile: &str, output: &str) -> Result<ExitStatus, Error> {
    let mut cmd = Command::new("aws");
    cmd.env("AWS_PROFILE", profile);
    cmd.args(vec![
        "s3api",
        "get-object",
        "--bucket",
        bucket,
        "--key",
        "kubeconfig",
        output,
    ]);
    Ok(cmd.status()?)
}
fn pause(msg: &'static str) {
    let theme = prompt_theme();
    Select::with_theme(&theme)
        .with_prompt(msg)
        .items(&["I'm done waiting"])
        .interact()
        .unwrap();
}

fn continue_prompt(msg: &'static str) -> bool {
    let theme = prompt_theme();
    Select::with_theme(&theme)
        .with_prompt(msg)
        .items(&["yes", "no"])
        .interact()
        .unwrap()
        == 0
}

fn prompt_theme() -> ColorfulTheme {
    ColorfulTheme {
        values_style: Style::new().yellow().dim(),
        indicator_style: Style::new().yellow().bold(),
        yes_style: Style::new().yellow().dim(),
        no_style: Style::new().yellow().dim(),
        ..ColorfulTheme::default()
    }
}

fn open_browser(url: &str) -> Result<(), Error> {
    // Conditional compilation: select the right open program for the OS.
    #[cfg(target_os = "macos")]
    let mut cmd = Command::new("open");
    #[cfg(target_os = "linux")]
    let mut cmd = Command::new("xdg-open");
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::null());
    cmd.arg(url);
    cmd.spawn()?;
    Ok(())
}

fn cluster_elbs_url(cluster_id: &str) -> String {
    format!("https://console.aws.amazon.com/ec2/home?region=us-east-1#LoadBalancers:tag:kubernetes.io/cluster/{}=*", cluster_id)
}

fn pick_cluster_id_prompt() -> Result<String, Error> {
    let theme = prompt_theme();
    let ids = valid_clusters();
    let idx = Select::with_theme(&theme)
        .with_prompt("Select a cluster id")
        .items(&ids)
        .interact()?;
    Ok(ids[idx].to_owned())
}

fn default_namespace(cluster_id: &str) -> &'static str {
    if cluster_id.starts_with("development") {
        return "development";
    }
    if cluster_id.starts_with("production") {
        return "production";
    }
    unreachable!("unknown cluster id {}", cluster_id)
}
