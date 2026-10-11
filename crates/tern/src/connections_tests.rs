#![allow(clippy::unwrap_used)]

use super::*;

fn draft(name: &str, host: &str, port: &str, user: &str) -> Draft {
    Draft {
        name: name.into(),
        host: host.into(),
        port: port.into(),
        user: user.into(),
        identity_file: String::new(),
        group: String::new(),
        tags: String::new(),
        vault_key: String::new(),
        ..Draft::default()
    }
}

#[test]
fn a_complete_draft_becomes_a_connection_with_port_22_by_default() {
    let c = validate(&draft(" web ", "10.0.0.5", "", "deploy"), &[], &[]).unwrap();
    assert_eq!(c.name, "web");
    assert_eq!(c.port, 22);
    assert_eq!(c.identity_file, None);
}

#[test]
fn a_vault_key_name_is_trimmed_kept_and_survives_the_file() {
    let mut d = draft("web", "10.0.0.5", "", "deploy");
    d.vault_key = "  laptop ".into();
    let c = validate(&d, &[], &[]).unwrap();
    assert_eq!(c.vault_key.as_deref(), Some("laptop"));
    let dir = std::env::temp_dir().join(format!("tern-conn-vk-{}", std::process::id()));
    save(&dir, std::slice::from_ref(&c)).unwrap();
    assert_eq!(load(&dir).unwrap(), vec![c]);
    let none = validate(&draft("web", "h", "", "u"), &[], &[]).unwrap();
    assert_eq!(none.vault_key, None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_older_hosts_file_loads_unchanged_and_round_trips_without_the_key() {
    let dir = tempfile::tempdir().unwrap();
    let old = r#"{"version":1,"connections":[{"name":"web","host":"h","port":22,"user":"u","vaultKey":"k"}]}"#;
    std::fs::write(dir.path().join(FILE_NAME), old).unwrap();
    let loaded = load(dir.path()).unwrap();
    assert_eq!(loaded[0].password_command, None);
    assert_eq!(loaded[0].vault_key.as_deref(), Some("k"));
    save(dir.path(), &loaded).unwrap();
    let text = std::fs::read_to_string(dir.path().join(FILE_NAME)).unwrap();
    assert!(!text.contains("passwordCommand"), "{text}");
    assert_eq!(load(dir.path()).unwrap(), loaded);
}

#[test]
fn a_password_command_is_trimmed_and_kept_under_its_camel_case_name() {
    let mut d = draft("web", "h", "", "u");
    d.password_command = "  gopass show -o work/web ".into();
    let c = validate(&d, &[], &[]).unwrap();
    assert_eq!(
        c.password_command.as_deref(),
        Some("gopass show -o work/web")
    );
    let dir = tempfile::tempdir().unwrap();
    save(dir.path(), std::slice::from_ref(&c)).unwrap();
    let text = std::fs::read_to_string(dir.path().join(FILE_NAME)).unwrap();
    assert!(
        text.contains(r#""passwordCommand": "gopass show -o work/web""#),
        "{text}"
    );
    assert_eq!(load(dir.path()).unwrap(), vec![c]);
    d.password_command = "   ".into();
    assert_eq!(validate(&d, &[], &[]).unwrap().password_command, None);
}

#[test]
fn a_sudo_password_command_is_optional_trimmed_and_named_in_camel_case() {
    let dir = tempfile::tempdir().unwrap();
    let old = r#"{"version":1,"connections":[{"name":"web","host":"h","port":22,"user":"u","passwordCommand":"p"}]}"#;
    std::fs::write(dir.path().join(FILE_NAME), old).unwrap();
    let loaded = load(dir.path()).unwrap();
    assert_eq!(loaded[0].sudo_password_command, None);
    save(dir.path(), &loaded).unwrap();
    let text = std::fs::read_to_string(dir.path().join(FILE_NAME)).unwrap();
    assert!(!text.contains("sudoPasswordCommand"), "{text}");

    let mut d = draft("web", "h", "", "u");
    d.sudo_password_command = "  gopass show -o sudo/web ".into();
    let c = validate(&d, &[], &[]).unwrap();
    assert_eq!(
        c.sudo_password_command.as_deref(),
        Some("gopass show -o sudo/web")
    );
    save(dir.path(), std::slice::from_ref(&c)).unwrap();
    let text = std::fs::read_to_string(dir.path().join(FILE_NAME)).unwrap();
    assert!(
        text.contains(r#""sudoPasswordCommand": "gopass show -o sudo/web""#),
        "{text}"
    );
    assert_eq!(load(dir.path()).unwrap(), vec![c]);
}

#[test]
fn each_bad_field_is_named() {
    assert!(
        validate(&draft("", "h", "", "u"), &[], &[])
            .unwrap_err()
            .contains("Name")
    );
    assert!(
        validate(&draft("a", "", "", "u"), &[], &[])
            .unwrap_err()
            .contains("Host")
    );
    assert!(
        validate(&draft("a", "h h", "", "u"), &[], &[])
            .unwrap_err()
            .contains("spaces")
    );
    assert!(
        validate(&draft("a", "h", "0", "u"), &[], &[])
            .unwrap_err()
            .contains("Port")
    );
    assert!(
        validate(&draft("a", "h", "70000", "u"), &[], &[])
            .unwrap_err()
            .contains("Port")
    );
    assert!(
        validate(&draft("a", "h", "x", "u"), &[], &[])
            .unwrap_err()
            .contains("Port")
    );
    assert!(
        validate(&draft("a", "h", "", " "), &[], &[])
            .unwrap_err()
            .contains("User")
    );
}

#[test]
fn names_are_unique_among_tern_connections() {
    let err = validate(&draft("web", "h", "", "u"), &["db", "web"], &[]).unwrap_err();
    assert!(err.contains("already exists"));
    assert!(validate(&draft("web", "h", "", "u"), &["db"], &[]).is_ok());
}

#[test]
fn missing_file_is_empty_and_save_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    assert!(load(dir.path()).unwrap().is_empty());
    let list = vec![validate(&draft("web", "10.0.0.5", "2222", "deploy"), &[], &[]).unwrap()];
    save(dir.path(), &list).unwrap();
    assert_eq!(load(dir.path()).unwrap(), list);
}

#[test]
fn a_broken_file_is_an_error_not_an_empty_list() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(FILE_NAME), "{ nope").unwrap();
    assert!(matches!(load(dir.path()), Err(StoreError::Unreadable(_))));
}

/// A crash between the temp write and the rename leaves a garbage `.tmp` beside the good
/// file; `load` reads the committed file and the next `save` replaces the stray.
#[test]
fn a_stray_tmp_beside_a_good_file_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let first = vec![conn("web", "10.0.0.5", "deploy")];
    save(dir.path(), &first).unwrap();
    std::fs::write(dir.path().join("hosts.json.tmp"), "{ half writ").unwrap();
    assert_eq!(load(dir.path()).unwrap(), first);

    let second = vec![
        conn("web", "10.0.0.5", "deploy"),
        conn("db", "10.0.0.6", "root"),
    ];
    save(dir.path(), &second).unwrap();
    assert_eq!(load(dir.path()).unwrap(), second);
}

/// The rename never happened: only the `.tmp` exists. It is not the hosts file.
#[test]
fn a_tmp_without_the_real_file_is_not_loaded() {
    let dir = tempfile::tempdir().unwrap();
    let tmp = r#"{"version":1,"connections":[{"name":"ghost","host":"h","port":22,"user":"u"}]}"#;
    std::fs::write(dir.path().join("hosts.json.tmp"), tmp).unwrap();
    assert!(load(dir.path()).unwrap().is_empty());
}

/// A file from a newer tern is refused, and left exactly as it was.
#[test]
fn a_newer_version_is_refused_and_left_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE_NAME);
    let newer = format!(
        r#"{{"version":{},"connections":[{{"name":"web","host":"h","port":22,"user":"u"}}]}}"#,
        VERSION + 1
    );
    std::fs::write(&path, &newer).unwrap();
    assert!(matches!(load(dir.path()), Err(StoreError::Unreadable(_))));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), newer);
}

#[test]
fn duplicating_an_ssh_config_host_keeps_its_fields() {
    let entry = HostEntry {
        alias: "grape".into(),
        host_name: "203.0.113.40".into(),
        port: 22,
        user: Some("root".into()),
        identity_files: vec![PathBuf::from("/k/id")],
        proxy_command: None,
        ..Default::default()
    };
    let c = Connection::from_entry(&entry);
    assert_eq!((c.name.as_str(), c.user.as_str()), ("grape", "root"));
    assert_eq!(c.identity_file.as_deref(), Some("/k/id"));
    assert_eq!(c.entry().host_name, "203.0.113.40");
    assert_eq!(c.entry().proxy_command, None);
}

/// A `hosts.json` written before groups and tags existed must load unchanged, and saving
/// it back must not invent the new fields.
#[test]
fn an_old_hosts_file_without_group_or_tags_loads_and_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let old = r#"{
  "version": 1,
  "connections": [
    { "name": "web", "host": "10.0.0.5", "port": 2222, "user": "deploy", "identityFile": "~/k" },
    { "name": "db", "host": "10.0.0.6", "port": 22, "user": "root" }
  ]
}"#;
    std::fs::write(dir.path().join(FILE_NAME), old).unwrap();
    let list = load(dir.path()).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].port, 2222);
    assert_eq!(list[0].identity_file.as_deref(), Some("~/k"));
    assert!(list.iter().all(|c| c.group.is_none() && c.tags.is_empty()));
    save(dir.path(), &list).unwrap();
    let text = std::fs::read_to_string(dir.path().join(FILE_NAME)).unwrap();
    assert!(!text.contains("group") && !text.contains("tags"), "{text}");
    assert_eq!(load(dir.path()).unwrap(), list);
}

#[test]
fn group_and_tags_survive_a_round_trip_and_are_cleaned() {
    let mut d = draft("web", "h", "", "u");
    d.group = "  Production ".into();
    d.tags = "eu, web ,,EU,db".into();
    let c = validate(&d, &[], &[]).unwrap();
    assert_eq!(c.group.as_deref(), Some("Production"));
    assert_eq!(c.tags, vec!["eu", "web", "db"]);
    let dir = tempfile::tempdir().unwrap();
    save(dir.path(), std::slice::from_ref(&c)).unwrap();
    assert_eq!(load(dir.path()).unwrap(), vec![c]);
    let blank = validate(&draft("web", "h", "", "u"), &[], &[]).unwrap();
    assert_eq!((blank.group, blank.tags), (None, Vec::new()));
}

/// A `hosts.json` written before jump hosts, forwards, agent and keep-alive existed.
const OLD_FILE: &str = r#"{
      "version": 1,
      "connections": [
        {"name": "web", "host": "10.0.0.5", "port": 2222, "user": "deploy",
         "identityFile": "~/.ssh/id_ed25519", "group": "prod", "tags": ["eu"],
         "proxyCommand": "ssh -W %h:%p bastion"}
      ]
    }"#;

#[test]
fn an_older_hosts_file_loads_unchanged_and_is_not_rewritten_with_new_fields() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(FILE_NAME), OLD_FILE).unwrap();
    let list = load(dir.path()).unwrap();
    assert_eq!(list.len(), 1);
    let c = &list[0];
    assert_eq!(
        (c.name.as_str(), c.port, c.group.as_deref()),
        ("web", 2222, Some("prod"))
    );
    assert_eq!(c.proxy_command.as_deref(), Some("ssh -W %h:%p bastion"));
    assert_eq!(c.proxy_jump, "");
    assert!(!c.forward_agent && c.forwards.is_empty() && c.keep_alive.is_none());
    // Saving it back adds none of the new keys.
    save(dir.path(), &list).unwrap();
    let text = std::fs::read_to_string(dir.path().join(FILE_NAME)).unwrap();
    for key in [
        "proxyJump",
        "forwardAgent",
        "keepAlive",
        "forwards",
        "agentSocket",
    ] {
        assert!(!text.contains(key), "{key} written for an old connection");
    }
    assert_eq!(
        c.entry().server_alive_interval,
        Some(std::time::Duration::from_secs(30))
    );
}

fn conn(name: &str, host: &str, user: &str) -> Connection {
    Connection {
        name: name.into(),
        host: host.into(),
        port: 22,
        user: user.into(),
        ..Connection::default()
    }
}

#[test]
fn a_jump_chain_resolves_saved_names_and_plain_targets_in_order() {
    let mut bastion = conn("bastion", "203.0.113.9", "ops");
    bastion.port = 2200;
    bastion.identity_file = Some("/k/bastion".into());
    let mut target = conn("db", "10.0.0.7", "root");
    target.proxy_jump = "bastion, deploy@gw.example.com:2022, [::1]:2300, 10.9.0.1".into();
    let all = vec![bastion, target.clone()];
    let hops = target.entry_in(&all).proxy_jump;
    assert_eq!(hops.len(), 4);
    assert_eq!(
        (hops[0].host.as_str(), hops[0].port, hops[0].user.as_deref()),
        ("203.0.113.9", 2200, Some("ops"))
    );
    assert_eq!(hops[0].identity_files, vec![PathBuf::from("/k/bastion")]);
    assert_eq!(
        (hops[1].host.as_str(), hops[1].port, hops[1].user.as_deref()),
        ("gw.example.com", 2022, Some("deploy"))
    );
    assert_eq!((hops[2].host.as_str(), hops[2].port), ("::1", 2300));
    assert_eq!(
        (hops[3].host.as_str(), hops[3].port, hops[3].user.clone()),
        ("10.9.0.1", 22, None)
    );
}

#[test]
fn a_bad_jump_host_is_refused_by_the_form_and_fails_by_name_if_it_is_stored() {
    let mut d = draft("web", "h", "", "u");
    for bad in ["a b", "h:99999", "h:0", "@h", "[::1"] {
        d.proxy_jump = bad.into();
        let e = validate(&d, &[], &[]).unwrap_err();
        assert!(e.contains("Jump host"), "{bad}: {e}");
    }
    let mut c = conn("web", "h", "u");
    c.proxy_jump = "ok.example.com, h:99999".into();
    let hops = c.entry().proxy_jump;
    assert_eq!(hops[0].port, 22);
    assert_eq!((hops[1].host.as_str(), hops[1].port), ("h:99999", 0));
}

#[test]
fn keep_alive_forward_agent_and_forwards_come_from_the_draft() {
    let mut d = draft("web", "h", "", "u");
    d.keep_alive = " 0 ".into();
    d.forward_agent = true;
    d.forwards = vec!["-L 8080:db:5432".into(), "  ".into(), "-D 1080".into()];
    let c = validate(&d, &[], &[]).unwrap();
    assert_eq!(c.keep_alive, Some(0));
    assert!(c.forward_agent);
    assert_eq!(
        c.forwards,
        ["-L 127.0.0.1:8080:db:5432", "-D 127.0.0.1:1080"]
    );
    let e = c.entry();
    assert_eq!(e.server_alive_interval, Some(std::time::Duration::ZERO));
    assert!(e.forward_agent);
    assert_eq!(e.forwards.len(), 2);
    d.keep_alive = "soon".into();
    assert!(validate(&d, &[], &[]).unwrap_err().contains("Keep-alive"));
    d.keep_alive = String::new();
    d.forwards = vec!["-L nope".into()];
    assert!(validate(&d, &[], &[]).unwrap_err().contains("Port forward"));
}

#[test]
fn importing_an_ssh_config_host_copies_jump_forwards_agent_and_keep_alive() {
    let entry = HostEntry {
        alias: "db".into(),
        host_name: "10.0.0.7".into(),
        port: 22,
        user: Some("root".into()),
        proxy_jump: vec![
            JumpHop {
                host: "bastion.example.com".into(),
                port: 2200,
                user: Some("ops".into()),
                identity_files: Vec::new(),
            },
            JumpHop {
                host: "10.1.1.1".into(),
                port: 22,
                ..JumpHop::default()
            },
        ],
        forwards: vec![
            Forward::parse_local("8080 db:5432").unwrap(),
            Forward::parse_dynamic("1080").unwrap(),
        ],
        forward_agent: true,
        agent_socket: Some(PathBuf::from("/tmp/agent.sock")),
        server_alive_interval: Some(std::time::Duration::from_secs(15)),
        ..HostEntry::default()
    };
    let c = Connection::from_entry(&entry);
    assert_eq!(c.proxy_jump, "ops@bastion.example.com:2200, 10.1.1.1");
    assert!(c.forward_agent);
    assert_eq!(c.keep_alive, Some(15));
    assert_eq!(
        c.forwards,
        ["-L 127.0.0.1:8080:db:5432", "-D 127.0.0.1:1080"]
    );
    let back = c.entry_in(&[]);
    assert_eq!(back.proxy_jump[0].host, "bastion.example.com");
    assert_eq!(back.proxy_jump[0].port, 2200);
    assert_eq!(back.proxy_jump[1].port, 22);
    assert_eq!(back.forwards, entry.forwards);
    assert_eq!(back.agent_socket, entry.agent_socket);
    assert_eq!(back.server_alive_interval, entry.server_alive_interval);
}
