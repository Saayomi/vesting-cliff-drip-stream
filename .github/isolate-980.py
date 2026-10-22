from pathlib import Path
import re
r=Path('.')
def item(text, marker):
    start=text.index(marker); opening=text.index('{',start); depth=1; i=opening+1
    while depth:
        if text[i]=='{': depth+=1
        elif text[i]=='}': depth-=1
        i+=1
    return text[start:i]
c=(r/'src/contract.rs').read_text();s=(r/'src/storage.rs').read_text();t=(r/'src/types.rs').read_text();e=(r/'src/events.rs').read_text();err=(r/'src/error.rs').read_text();tests=(r/'src/tests/test_drain.rs').read_text()
out=Path('/tmp/drain-isolated');(out/'src/tests').mkdir(parents=True,exist_ok=True)
(out/'Cargo.toml').write_text('[package]\nname="drain-isolated"\nversion="0.1.0"\nedition="2021"\n[dependencies]\nsoroban-sdk={version="22.0.0",features=["testutils"]}\n')
(out/'src/lib.rs').write_text('mod contract; mod error; mod events; mod storage; mod types; mod migration; #[cfg(test)] mod tests;')
(out/'src/contract.rs').write_text('use soroban_sdk::{contract,contractimpl,token,Address,Env}; use crate::{error::VestingError,events,storage,types::RATE_DECIMALS};\n'+re.search(r'pub\(crate\) const DRAIN_DELAY_LEDGERS[^;]+;',c).group()+'\n#[contract] pub struct VestingDrips; #[contractimpl] impl VestingDrips {\n'+item(c,'pub fn drain_expired_stream(')+'\n}\n'+item(c,'pub fn calculate_total_deposit('))
variants=[]
for name in ['ScheduleNotFound','StreamNotExpired','DrainDelayNotExpired','DepositOverflow','TransferFailed']:
 variants.append(re.search(r'\b'+name+r' = \d+',err).group())
(out/'src/error.rs').write_text('use soroban_sdk::contracterror; #[contracterror] #[derive(Copy,Clone,Debug,Eq,PartialEq)] #[repr(u32)] pub enum VestingError {'+','.join(variants)+'}')
(out/'src/types.rs').write_text('use soroban_sdk::{contracttype,Address,String}; pub const RATE_DECIMALS:i128=10_000_000; pub const CURRENT_SCHEMA_VERSION:u32=2; #[contracttype] #[derive(Clone,Debug)] '+item(t,'pub struct VestingSchedule')+'\n#[contracttype] #[derive(Clone)] pub enum DataKey {Schedule(Address),ScheduleById(Address,u32),NextStreamId(Address),SponsorStreams(Address)}')
functions=['bump_persistent','compute_stream_ttl','ensure_ttl_for_stream','ensure_ttl_for_stream_id','get_schedule','set_schedule','set_schedule_by_id','remove_schedule','remove_schedule_by_id','get_sponsor_streams','remove_sponsor_stream']
parts=[]
for name in functions:
 marker='pub fn '+name+(' <' if False else '')
 # Opening parenthesis disambiguates names; generic bump helper uses '<'.
 marker += '<' if name=='bump_persistent' else '('
 parts.append(item(s,marker))
(out/'src/storage.rs').write_text('use soroban_sdk::{Env,Address,Vec}; use crate::types::{DataKey,VestingSchedule}; const PERSISTENT_LEDGER_THRESHOLD:u32=3_000_000; const PERSISTENT_BUMP_AMOUNT:u32=3_110_400; const TTL_BUFFER_LEDGERS:u32=3_153_600;\n'+'\n'.join(parts))
(out/'src/migration.rs').write_text((r/'src/migration.rs').read_text())
(out/'src/events.rs').write_text('use soroban_sdk::{Env,Address,symbol_short};'+item(e,'pub fn emit_stream_drained('))
(out/'src/tests/mod.rs').write_text('pub mod token_helper; mod test_drain;')
(out/'src/tests/token_helper.rs').write_text((r/'src/tests/token_helper.rs').read_text())
prefix=tests[:tests.index('/// Creates a fixture')]
(out/'src/tests/test_drain.rs').write_text(prefix+item(tests,'pub(super) fn setup_env(')+'\n'+tests[tests.index('// Direct fixtures'):])
