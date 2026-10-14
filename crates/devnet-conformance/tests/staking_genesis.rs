//! Staking/distribution/slashing in isolation from the ante handler:
//! replays the `empty` scenario's genesis (delivering the gentx's
//! `MsgCreateValidator` straight to the staking msg server) and block 1's
//! begin/end blockers, then compares the module stores with Go's dump.

use celestia_proto::celestia_core_proto::abci::{request, response};
use celestia_proto::cosmos::staking::v1beta1::MsgCreateValidator;
use devnet_conformance::{load_exchanges, load_genesis, load_state};
use sdk::{Context, ExecMode, Header, MultiStore};

const STORES: &[&str] = &["staking", "distribution", "slashing"];

fn hexs(b: &[u8]) -> String {
    hex::encode(b)
}

#[test]
#[allow(clippy::too_many_lines)]
fn module_stores_match_go_at_height_1() -> anyhow::Result<()> {
    let exchanges = load_exchanges("empty")?;
    let mut init = None;
    let mut init_resp = None;
    let mut block1 = None;
    for ex in exchanges {
        match (ex.request.value, ex.response.value) {
            (Some(request::Value::InitChain(r)), Some(response::Value::InitChain(g))) => {
                init = Some(r);
                init_resp = Some(g);
            }
            (Some(request::Value::FinalizeBlock(r)), _) if r.height == 1 => block1 = Some(r),
            _ => {}
        }
    }
    let (init, init_resp, block1) = (
        init.ok_or_else(|| anyhow::anyhow!("no InitChain"))?,
        init_resp.ok_or_else(|| anyhow::anyhow!("no InitChain response"))?,
        block1.ok_or_else(|| anyhow::anyhow!("no block 1"))?,
    );
    let genesis = load_genesis("empty")?;
    let app_state = &genesis["app_state"];

    let mut ms = MultiStore::new();
    for (name, kind) in app::stores::all() {
        ms.mount(name, kind)?;
    }
    app::modules::install_staking_hooks();

    // InitChain.
    let header = Header {
        chain_id: init.chain_id.clone(),
        height: 0,
        time: init.time.unwrap_or_default(),
        ..Header::default()
    };
    let mut ctx = Context::new(&mut ms, header, ExecMode::Finalize);
    ctx.consensus_params = init.consensus_params.clone().unwrap_or_default();
    let mut validators = Vec::new();
    for module in app::modules::INIT_GENESIS {
        if *module == "genutil" {
            for tx in app_state["genutil"]["gen_txs"]
                .as_array()
                .into_iter()
                .flatten()
            {
                for m in tx["body"]["messages"].as_array().into_iter().flatten() {
                    let mut m = m.clone();
                    x_staking::msgs::json_msg_to_wire(&mut m)?;
                    if let Some(o) = m.as_object_mut() {
                        o.remove("@type");
                    }
                    let msg: MsgCreateValidator = serde_json::from_value(m)?;
                    let cp = ctx.branch();
                    x_staking::handle_create_validator(&mut ctx, &msg)?;
                    ctx.write(cp);
                }
            }
            validators = x_staking::apply_and_return_validator_set_updates(&mut ctx)?;
            continue;
        }
        app::modules::init_genesis(&mut ctx, module, &app_state[*module])?;
    }
    assert_eq!(validators, init_resp.validators, "InitChain validator set");
    drop(ctx);

    // FinalizeBlock(1): begin and end blockers.
    let header = Header {
        chain_id: init.chain_id.clone(),
        height: 1,
        time: block1.time.unwrap_or_default(),
        proposer_address: block1.proposer_address.clone(),
        hash: block1.hash.clone(),
        app_hash: hex::decode("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")?,
        next_validators_hash: block1.next_validators_hash.clone(),
    };
    let mut ctx = Context::new(&mut ms, header, ExecMode::Finalize);
    ctx.consensus_params = init.consensus_params.clone().unwrap_or_default();
    ctx.vote_infos = block1
        .decided_last_commit
        .as_ref()
        .map(|c| c.votes.clone())
        .unwrap_or_default();
    for module in app::modules::BEGIN_BLOCKERS {
        app::modules::begin_block(&mut ctx, module)?;
    }
    for module in app::modules::END_BLOCKERS {
        app::modules::end_block(&mut ctx, module)?;
    }
    drop(ctx);
    ms.flush()?;
    ms.commit()?;

    let dump = load_state("empty", 1)?;
    let mut failures = Vec::new();
    for name in STORES {
        let go = dump
            .stores
            .iter()
            .find(|s| s.name == *name)
            .ok_or_else(|| anyhow::anyhow!("no {name} in dump"))?;
        let ours: Vec<(Vec<u8>, Vec<u8>)> = ms
            .root()
            .get_store(name)?
            .iter(None, None)?
            .map(|e| (e.key, e.value))
            .collect();
        let theirs: Vec<(Vec<u8>, Vec<u8>)> =
            go.kv.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        assert!(!theirs.is_empty(), "{name} is empty in the Go dump");
        if ours != theirs {
            for (k, v) in &theirs {
                match ours.iter().find(|(ok, _)| ok == k) {
                    None => failures.push(format!("{name}: missing {}", hexs(k))),
                    Some((_, ov)) if ov != v => failures.push(format!(
                        "{name}: {} go={} rust={}",
                        hexs(k),
                        hexs(v),
                        hexs(ov)
                    )),
                    Some(_) => {}
                }
            }
            for (k, _) in &ours {
                if !theirs.iter().any(|(tk, _)| tk == k) {
                    failures.push(format!("{name}: extra {}", hexs(k)));
                }
            }
        } else if ms.root().get_store(name)?.root_hash().as_slice() != go.hash.as_slice() {
            failures.push(format!("{name}: kv equal but hash differs"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
    Ok(())
}
