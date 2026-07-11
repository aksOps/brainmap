#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
producer="${root}/scripts/m8-codex-fia5.sh"
temporary="$(mktemp -d)"
cleanup_test() {
  if [[ "${FIA5_TEST_KEEP:-0}" == 1 ]]; then
    printf 'retained FIA-5 test workspace: %s\n' "${temporary}" >&2
  else
    rm -rf "${temporary}"
  fi
}
trap cleanup_test EXIT
checks=0

fail() {
  echo "m8 Codex FIA-5 producer test failed: $*" >&2
  exit 1
}

pass() {
  checks=$((checks + 1))
}

expect_failure() {
  local expected="$1"
  shift
  local output
  if output="$("$@" 2>&1)"; then
    fail "command unexpectedly succeeded; expected: ${expected}"
  fi
  grep -F -- "${expected}" <<<"${output}" >/dev/null ||
    fail "missing error '${expected}' in: ${output}"
  pass
}

[[ -x "${producer}" ]] || fail "producer is not executable"
pass

help="$(${producer} --help)"
for required in \
  'prepare --brainmap PATH --brainmapd PATH --candidate-commit COMMIT --codex-archive PATH --state DIR' \
  'prepare --fixture --brainmap PATH --brainmapd PATH --candidate-commit COMMIT --state DIR' \
  'begin --state DIR' \
  'finalize --state DIR --out DIR'; do
  grep -F -- "${required}" <<<"${help}" >/dev/null ||
    fail "help is missing ${required}"
  pass
done
grep -F -- 'qualificationEligible:false' <<<"${help}" >/dev/null ||
  fail 'help does not label fixture evidence as non-qualifying'
pass
grep -E -- '--(fake|test|local|diagnostic|trust-bypass|event|success)([[:space:]]|$)' \
  <<<"${help}" >/dev/null && fail 'help exposes a caller-asserted qualification mode'
pass

fixture="${temporary}/fixture"
mkdir -p "${fixture}/scripts" "${fixture}/bin"
cp "${producer}" "${fixture}/scripts/m8-codex-fia5.sh"
chmod 0755 "${fixture}/scripts/m8-codex-fia5.sh"

cat >"${fixture}/bin/brainmap" <<'BRAINMAP'
#!/usr/bin/env bash
set -euo pipefail

value_after() {
  local expected="$1"
  shift
  while (($#)); do
    if [[ "$1" == "${expected}" ]]; then
      printf '%s\n' "$2"
      return 0
    fi
    shift
  done
  return 1
}

case "${1:-} ${2:-}" in
  'init-vault --vault')
    vault="$3"
    mkdir -p "${vault}/90-calibration" "${vault}/.brainmap"
    : >"${vault}/90-calibration/decision-ledger.jsonl"
    printf '{"valid":true}\n' >"${vault}/.brainmap/index-manifest.json"
    ;;
  'index rebuild')
    vault="$(value_after --vault "$@")"
    mkdir -p "${vault}/.brainmap"
    printf '{"valid":true}\n' >"${vault}/.brainmap/index-manifest.json"
    ;;
  'gate-mode active')
    vault="$(value_after --vault "$@")"
    printf '{"gateMode":"active","mode":"shadow","killSwitch":false}\n' \
      >"${vault}/.brainmap/engine.json"
    ;;
  'autopilot demote')
    [[ "$(value_after --to "$@")" == conservative ]] || exit 89
    vault="$(value_after --vault "$@")"
    printf '{"gateMode":"active","mode":"conservative","killSwitch":false}\n' \
      >"${vault}/.brainmap/engine.json"
    ;;
  'autopilot status')
    vault="$(value_after --vault "$@")"
    if [[ "${FIA5_FIXTURE_BAD_ENGINE:-0}" == 1 ]]; then
      printf '{"gateMode":"shadow","mode":"shadow","killSwitch":false}\n'
    else
      cat "${vault}/.brainmap/engine.json"
    fi
    ;;
  'install harness')
    project="$(value_after --project "$@")"
    vault="$(value_after --vault "$@")"
    self="$(cd "$(dirname "$0")" && pwd -P)/$(basename "$0")"
    if [[ " $* " == *' --dry-run '* ]]; then
      printf '%s\n' \
        'install harness dry-run target=codex' \
        "would create ${project}/.codex/skills/build-decision-engine/SKILL.md (instruction-only)" \
        "would create ${project}/AGENTS.md (instruction-only)" \
        "would create ${project}/.codex/config.toml (best-effort)" \
        "would create ${project}/.codex/hooks.json (enforced)"
      exit 0
    fi
    mkdir -p "${project}/.codex/skills/build-decision-engine"
    printf '# synthetic skill\n' >"${project}/.codex/skills/build-decision-engine/SKILL.md"
    printf '# synthetic Brainmap instructions\n' >"${project}/AGENTS.md"
    cat >"${project}/.codex/config.toml" <<EOF
# BEGIN BRAINMAP MANAGED BLOCK
[mcp_servers.brainmap]
command = "${self}"
args = ["mcp", "serve", "--vault", "${vault}"]
required = true
default_tools_approval_mode = "auto"
enabled_tools = ["brainmap_decision_gate", "brainmap_record_decision", "brainmap_learn_feedback", "brainmap_preview_update", "brainmap_apply_update"]

[mcp_servers.brainmap.tools.brainmap_learn_feedback]
approval_mode = "prompt"

[mcp_servers.brainmap.tools.brainmap_apply_update]
approval_mode = "prompt"
# END BRAINMAP MANAGED BLOCK
EOF
    jq -n \
      --arg prompt "'${self}' harness hook --host codex --event UserPromptSubmit" \
      --arg tool "'${self}' harness hook --host codex --event PreToolUse" '{
      hooks: {
        UserPromptSubmit: [{hooks:[{type:"command",command:$prompt,timeout:10}]}],
        PreToolUse: [{
          matcher:"Bash|Edit|Write|MultiEdit|NotebookEdit",
          hooks:[{type:"command",command:$tool,timeout:10}]
        }]
      }
    }' >"${project}/.codex/hooks.json"
    printf '%s\n' \
      "wrote ${project}/.codex/skills/build-decision-engine/SKILL.md (instruction-only)" \
      "wrote ${project}/AGENTS.md (instruction-only)" \
      "wrote ${project}/.codex/config.toml (best-effort)" \
      "wrote ${project}/.codex/hooks.json (enforced)"
    ;;
  'integration doctor')
    jq -n '{
      target:"codex",supported:true,installed:true,configurationValid:true,
      executableAvailable:true,vaultExists:true,indexValid:true,gateReachable:true,
      recordingSupported:true,feedbackSupported:true,activationRequiresApproval:true,
      mcpVaultConfigured:true,projectTrustRequired:true,projectTrusted:true,
      projectTrustConfigurationValid:true,projectTrustError:null,
      healthScope:"local-adapter-files-and-contract",hostHookTrustVerified:false,
      hostProbeRequired:true,
      enforcement:["instruction-only","instruction-only","best-effort","enforced"],
      healthy:true
    }'
    ;;
  *)
    echo "unexpected brainmap stub invocation: $*" >&2
    exit 90
    ;;
esac
BRAINMAP
chmod 0755 "${fixture}/bin/brainmap"

cat >"${fixture}/bin/brainmapd" <<'BRAINMAPD'
#!/usr/bin/env bash
exit 0
BRAINMAPD
chmod 0755 "${fixture}/bin/brainmapd"

cat >"${fixture}/bin/codex" <<'CODEX'
#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == --version ]]; then
  printf 'codex-cli 0.144.0\n'
  exit 0
fi

state="${FIA5_FIXTURE_STATE:?}"
manifest="${state}/state.json"
project="$(jq -er '.paths.project' "${manifest}")"
vault="$(jq -er '.paths.vault' "${manifest}")"
brainmap="$(jq -er '.paths.brainmap' "${manifest}")"
expected_home="$(jq -er '.paths.codexHome' "${manifest}")"
thread_id='019f5000-0000-7000-8000-000000000001'
decision_id='dec_1720000000000_aaaaaaaaaaaa'
second_decision_id='dec_1720000000002_cccccccccccc'
packet_id='upd_1720000000001_bbbbbbbbbbbb'
[[ "${FIA5_FIXTURE_BAD_RUNTIME_ID:-0}" != 1 ]] || decision_id='dec_fixture_first'

if [[ "$#" -eq 10 && "$9" == app-server && "${10}" == --stdio ]]; then
  [[ "$1" == -c && "$2" == 'approval_policy="on-request"' ]] || exit 71
  [[ "$3" == -c && "$4" == 'approvals_reviewer="user"' ]] || exit 72
  [[ "$5" == -c && "$6" == 'sandbox_mode="workspace-write"' ]] || exit 73
  [[ "$7" == -c && "$8" == 'sandbox_workspace_write.network_access=false' ]] || exit 74
  [[ "${CODEX_HOME:?}" == "${expected_home}" ]] || exit 75
  while IFS= read -r request; do
    method="$(jq -r '.method' <<<"${request}")"
    id="$(jq -r '.id // empty' <<<"${request}")"
    case "${method}" in
      initialize)
        jq -nc --argjson id "${id}" --arg home "${CODEX_HOME}" \
          '{id:$id,result:{codexHome:$home,userAgent:"brainmap-fia5/0.144.0"}}'
        ;;
      initialized)
        ;;
      config/read)
        approval='on-request'
        reviewer='user'
        sandbox='workspace-write'
        bypass=false
        network=false
        [[ "${FIA5_FIXTURE_UNSAFE_APPROVAL:-0}" != 1 ]] || approval='never'
        [[ "${FIA5_FIXTURE_UNSAFE_REVIEWER:-0}" != 1 ]] || reviewer='auto_review'
        [[ "${FIA5_FIXTURE_UNSAFE_SANDBOX:-0}" != 1 ]] || sandbox='danger-full-access'
        [[ "${FIA5_FIXTURE_BYPASS_CONFIG:-0}" != 1 ]] || bypass=true
        [[ "${FIA5_FIXTURE_MISSING_NETWORK:-0}" != 1 ]] || network=null
        jq -nc \
          --argjson id "${id}" \
          --arg approval "${approval}" \
          --arg reviewer "${reviewer}" \
          --arg sandbox "${sandbox}" \
          --argjson bypass "${bypass}" \
          --argjson network "${network}" \
          --arg brainmap "${brainmap}" \
          --arg vault "${vault}" '{
            id:$id,
            result:{config:{
              approval_policy:$approval,
              approvals_reviewer:$reviewer,
              sandbox_mode:$sandbox,
              sandbox_workspace_write:{network_access:$network},
              bypass_hook_trust:$bypass,
              mcp_servers:{brainmap:{
                command:$brainmap,
                args:["mcp","serve","--vault",$vault],
                required:true,
                default_tools_approval_mode:"auto",
                tools:{
                  brainmap_learn_feedback:{approval_mode:"prompt"},
                  brainmap_apply_update:{approval_mode:"prompt"}
                }
              }}
            },layers:[]}
          }'
        ;;
      hooks/list)
        trust='trusted'
        [[ "${FIA5_FIXTURE_UNTRUSTED:-0}" != 1 ]] || trust='untrusted'
        prompt_hash="sha256:$(printf 'a%.0s' {1..64})"
        tool_hash="sha256:$(printf 'b%.0s' {1..64})"
        jq -nc \
          --argjson id "${id}" \
          --arg project "${project}" \
          --arg prompt "'${brainmap}' harness hook --host codex --event UserPromptSubmit" \
          --arg tool "'${brainmap}' harness hook --host codex --event PreToolUse" \
          --arg trust "${trust}" \
          --arg promptHash "${prompt_hash}" \
          --arg toolHash "${tool_hash}" \
          --argjson extraHook "${FIA5_FIXTURE_EXTRA_HOOK:-0}" '{
            id:$id,
            result:{data:[{
              cwd:$project,warnings:[],errors:[],hooks:([
                {
                  key:"fixture-prompt",eventName:"userPromptSubmit",handlerType:"command",
                  command:$prompt,matcher:null,currentHash:$promptHash,displayOrder:0,
                  enabled:true,isManaged:false,source:"project",sourcePath:$project,
                  timeoutSec:10,trustStatus:$trust,statusMessage:null,pluginId:null
                },
                {
                  key:"fixture-tool",eventName:"preToolUse",handlerType:"command",
                  command:$tool,matcher:"Bash|Edit|Write|MultiEdit|NotebookEdit",
                  currentHash:$toolHash,displayOrder:1,enabled:true,isManaged:false,
                  source:"project",sourcePath:$project,timeoutSec:10,
                  trustStatus:$trust,statusMessage:null,pluginId:null
                }
              ] + (if $extraHook == 1 then [{
                key:"fixture-extra",eventName:"userPromptSubmit",handlerType:"command",
                command:"/bin/true",matcher:null,currentHash:$promptHash,displayOrder:2,
                enabled:true,isManaged:false,source:"user",sourcePath:"redacted",
                timeoutSec:10,trustStatus:"trusted",statusMessage:null,pluginId:null
              }] else [] end))
            }]}
          }'
        ;;
      thread/list)
        if [[ ! -f "${state}/private/launch-marker.json" ]]; then
          jq -nc --argjson id "${id}" \
            '{id:$id,result:{data:[],nextCursor:null,backwardsCursor:null}}'
          continue
        fi
        created="$(jq -er '.startedAtEpoch' "${state}/private/launch-marker.json")"
        extra_thread=false
        if [[ "${FIA5_FIXTURE_LATE_EXTRA_THREAD:-0}" == 1 ]]; then
          late_count_file="${state}/private/late-thread-list-count"
          late_count=0
          [[ ! -f "${late_count_file}" ]] || late_count="$(<"${late_count_file}")"
          late_count=$((late_count + 1))
          printf '%s\n' "${late_count}" >"${late_count_file}"
          [[ "${late_count}" -lt 2 ]] || extra_thread=true
        fi
        jq -nc \
          --argjson id "${id}" \
          --arg threadId "${thread_id}" \
          --arg project "${project}" \
          --argjson created "${created}" \
          --argjson extraThread "${extra_thread}" '{
            id:$id,
            result:{data:([{
              id:$threadId,sessionId:$threadId,source:"cli",cwd:$project,
              cliVersion:"0.144.0",ephemeral:false,parentThreadId:null,
              createdAt:$created,updatedAt:($created+1),modelProvider:"openai",
              preview:"not retained by producer",status:"notLoaded",turns:[]
            }] + (if $extraThread then [{
              id:"019f5000-0000-7000-8000-000000000002",
              sessionId:"019f5000-0000-7000-8000-000000000002",
              source:"cli",cwd:$project,cliVersion:"0.144.0",ephemeral:false,
              parentThreadId:null,createdAt:$created,updatedAt:($created+1),
              modelProvider:"openai",preview:"extra",status:"notLoaded",turns:[]
            }] else [] end)),nextCursor:null,backwardsCursor:null}
          }'
        ;;
      thread/read)
        created="$(jq -er '.startedAtEpoch' "${state}/private/launch-marker.json")"
        jq -nc \
          --argjson id "${id}" \
          --arg threadId "${thread_id}" \
          --arg project "${project}" \
          --arg decisionId "${decision_id}" \
          --arg secondDecisionId "${second_decision_id}" \
          --arg packetId "${packet_id}" \
          --argjson created "${created}" \
          --argjson badOrder "${FIA5_FIXTURE_BAD_ORDER:-0}" \
          --argjson badSecondOutcome "${FIA5_FIXTURE_BAD_SECOND_OUTCOME:-0}" \
          --argjson badSecondRecord "${FIA5_FIXTURE_BAD_SECOND_RECORD:-0}" \
          --argjson droppedRejected "${FIA5_FIXTURE_DROPPED_REJECTED:-0}" \
          --argjson sideEffect "${FIA5_FIXTURE_SIDE_EFFECT_ITEM:-0}" \
          --argjson extraTool "${FIA5_FIXTURE_EXTRA_TOOL:-0}" \
          --argjson secondUser "${FIA5_FIXTURE_SECOND_USER:-0}" '
          def text_result($value): {content:[{type:"text",text:($value|tojson)}]};
          def call($id;$tool;$arguments;$value): {
            type:"mcpToolCall",id:$id,server:"brainmap",tool:$tool,
            status:"completed",arguments:$arguments,error:null,
            result:text_result($value)
          };
          ({
            intent:"would-ask-user",
            situation:"Choose formatter for synthetic FIA-5 project",
            options:["biome","prettier"],risk:"low",reversible:true,
            decisionType:"tooling",scope:"project:fia5",dryRun:false
          }) as $gateArgs
          | ({
              decisionId:$secondDecisionId,
              outcome:(if $badSecondOutcome == 1 then "ask_user" else "proceed" end),
              selectedOption:(if $badSecondOutcome == 1 then null else "prettier" end),
              predictedOutcome:"proceed",predictedSelectedOption:"prettier",
              gateMode:"active",autopilotMode:"conservative"
            }) as $secondResult
          | ([
              call("call-1";"brainmap_decision_gate";$gateArgs;{
                decisionId:$decisionId,outcome:"ask_user",selectedOption:null,
                predictedOutcome:"ask_user",predictedSelectedOption:null,
                gateMode:"active",autopilotMode:"conservative"
              }),
              call("call-2";"brainmap_record_decision";{
                decisionId:$decisionId,chosen:"biome",wasAsked:true
              };{recorded:true}),
              call("call-3";"brainmap_learn_feedback";{
                decisionId:$decisionId,chosen:"prettier",rejected:["biome"]
              };{packetCreated:true,packetId:$packetId}),
              call("call-4";"brainmap_preview_update";{packetId:$packetId};[{
                id:$packetId,status:"pending",decisionRule:{
                  chosen:"prettier",
                  rejected:(if $droppedRejected == 1 then [] else ["biome"] end)
                }
              }]),
              call("call-5";"brainmap_apply_update";{
                packetId:$packetId,approved:true
              };{applied:true,packetId:$packetId}),
              call("call-6";"brainmap_decision_gate";$gateArgs;$secondResult),
              call("call-7";"brainmap_record_decision";{
                decisionId:$secondDecisionId,
                chosen:(if $badSecondRecord == 1 then "biome" else "prettier" end),
                wasAsked:(if $badSecondRecord == 1 then true else false end)
              };{recorded:true})
            ]) as $calls
          | (if $badOrder == 1 then
               [$calls[0],$calls[2],$calls[1],$calls[3],$calls[4],$calls[5],$calls[6]]
             else $calls end) as $ordered
          | ([
              {type:"userMessage",id:"user-1",content:[]},
              $ordered[0],
              {type:"userMessage",id:"user-2",content:[]},
              $ordered[1],$ordered[2],$ordered[3],$ordered[4],$ordered[5]
            ]
            + (if $secondUser == 1 then [{type:"userMessage",id:"user-3",content:[]}]
               else [] end)
            + [$ordered[6]]
            + (if $sideEffect == 1 then [{
                type:"commandExecution",id:"shell-1",command:"true",status:"completed"
              }] else [] end)
            + (if $extraTool == 1 then [
                call("call-8";"brainmap_unexpected";{};{ok:true})
              ] else [] end)) as $items
          | {
              id:$id,
              result:{thread:{
                id:$threadId,sessionId:$threadId,source:"cli",cwd:$project,
                cliVersion:"0.144.0",ephemeral:false,parentThreadId:null,
                createdAt:$created,updatedAt:($created+1),modelProvider:"openai",
                preview:"private",status:"notLoaded",
                turns:[{id:"turn-1",status:"completed",items:$items}]
              }}
            }
        '
        ;;
      *)
        jq -nc --argjson id "${id:-0}" \
          '{id:$id,error:{code:-32601,message:"unsupported"}}'
        ;;
    esac
  done
  exit 0
fi

[[ "$#" -eq 12 ]] || exit 81
[[ "$1" == --ask-for-approval && "$2" == on-request ]] || exit 82
[[ "$3" == --sandbox && "$4" == workspace-write ]] || exit 83
[[ "$5" == -c && "$6" == 'approvals_reviewer="user"' ]] || exit 84
[[ "$7" == -c && "$8" == 'sandbox_workspace_write.network_access=false' ]] || exit 85
[[ "$9" == --cd && "${10}" == "${project}" && "${11}" == --no-alt-screen ]] || exit 86
[[ "${CODEX_HOME:?}" == "${expected_home}" ]] || exit 87
for argument in "$@"; do
  [[ "${argument}" != --dangerously-bypass-hook-trust ]] || exit 88
  [[ "${argument}" != --dangerously-bypass-approvals-and-sandbox ]] || exit 89
  [[ "${argument}" != danger-full-access && "${argument}" != never ]] || exit 90
done

ledger="${vault}/90-calibration/decision-ledger.jsonl"
jq -nc '{
  id:"dec_1720000000099_dddddddddddd",createdAt:"2026-07-10T17:00:00Z",
  kind:"decision-gate",intent:"agent-hook:UserPromptSubmit",
  situation:"synthetic hook event",options:[],decisionType:"agent-harness",
  scope:"project:fia5",outcome:"ask_user",selectedOption:null,
  predictedOutcome:"ask_user",predictedSelectedOption:null,
  gateMode:"active",autopilotMode:"conservative"
}' >>"${ledger}"
jq -nc --arg decisionId 'dec_1720000000000_aaaaaaaaaaaa' '{
  id:$decisionId,createdAt:"2026-07-10T17:00:01Z",kind:"decision-gate",
  intent:"would-ask-user",situation:"Choose formatter for synthetic FIA-5 project",
  options:["biome","prettier"],risk:"low",reversible:true,
  decisionType:"tooling",scope:"project:fia5",
  outcome:"ask_user",selectedOption:null,predictedOutcome:"ask_user",
  predictedSelectedOption:null,gateMode:"active",autopilotMode:"conservative"
}' >>"${ledger}"
jq -nc --arg decisionId 'dec_1720000000000_aaaaaaaaaaaa' '{
  id:"action_1720000000000_aaaaaaaaaaaa",decisionId:$decisionId,
  createdAt:"2026-07-10T17:00:02Z",kind:"record-decision",
  chosen:"biome",wasAsked:true
}' >>"${ledger}"
jq -nc \
  --arg decisionId 'dec_1720000000000_aaaaaaaaaaaa' \
  --arg packetId 'upd_1720000000001_bbbbbbbbbbbb' '{
  id:"feedback_1720000000001_bbbbbbbbbbbb",decisionId:$decisionId,
  packetId:$packetId,createdAt:"2026-07-10T17:00:03Z",kind:"learn-feedback",
  chosen:"prettier",rejected:["biome"]
}' >>"${ledger}"
jq -nc --arg decisionId 'dec_1720000000002_cccccccccccc' '{
  id:$decisionId,createdAt:"2026-07-10T17:00:04Z",kind:"decision-gate",
  intent:"would-ask-user",situation:"Choose formatter for synthetic FIA-5 project",
  options:["biome","prettier"],risk:"low",reversible:true,
  decisionType:"tooling",scope:"project:fia5",
  outcome:"proceed",selectedOption:"prettier",predictedOutcome:"proceed",
  predictedSelectedOption:"prettier",gateMode:"active",autopilotMode:"conservative"
}' >>"${ledger}"
jq -nc --arg decisionId 'dec_1720000000002_cccccccccccc' '{
  id:"action_1720000000002_cccccccccccc",decisionId:$decisionId,
  createdAt:"2026-07-10T17:00:05Z",kind:"record-decision",
  chosen:"prettier",wasAsked:false
}' >>"${ledger}"
CODEX
chmod 0755 "${fixture}/bin/codex"

git -C "${fixture}" init -q
git -C "${fixture}" config user.email fixture@example.invalid
git -C "${fixture}" config user.name Fixture
git -C "${fixture}" add .
git -C "${fixture}" commit -qm 'strict Codex FIA-5 fixture'

candidate_commit="$(git -C "${fixture}" rev-parse HEAD)"
brainmap="${fixture}/bin/brainmap"
brainmapd="${fixture}/bin/brainmapd"
fixture_producer="${fixture}/scripts/m8-codex-fia5.sh"
codex_home="${temporary}/codex-home"
state="${temporary}/fia5-state"
out="${temporary}/fia5-evidence"
mkdir -p "${codex_home}"
export PATH="${fixture}/bin:${PATH}"
export CODEX_HOME="${codex_home}"
export FIA5_FIXTURE_STATE="${state}"

expect_failure 'unknown prepare argument: --fake' "${fixture_producer}" prepare --fake
expect_failure 'qualification prepare requires --codex-archive PATH' \
  "${fixture_producer}" prepare \
  --brainmap "${brainmap}" --brainmapd "${brainmapd}" \
  --candidate-commit "${candidate_commit}" --state "${temporary}/missing-archive-state"
printf 'not the official archive\n' >"${temporary}/bogus-codex.tar.gz"
expect_failure 'pinned official 0.144.0' \
  "${fixture_producer}" prepare \
  --brainmap "${brainmap}" --brainmapd "${brainmapd}" \
  --candidate-commit "${candidate_commit}" \
  --codex-archive "${temporary}/bogus-codex.tar.gz" \
  --state "${temporary}/bogus-archive-state"

touch "${fixture}/dirty"
expect_failure 'clean candidate HEAD' \
  "${fixture_producer}" prepare --fixture \
  --brainmap "${brainmap}" --brainmapd "${brainmapd}" \
  --candidate-commit "${candidate_commit}" --state "${temporary}/dirty-state"
rm "${fixture}/dirty"

"${fixture_producer}" prepare --fixture \
  --brainmap "${brainmap}" \
  --brainmapd "${brainmapd}" \
  --candidate-commit "${candidate_commit}" \
  --state "${state}" >/dev/null
jq -e '
  .schemaVersion == "brainmap-m8-codex-fia5-state-v2"
  and .mode == "fixture"
  and .qualificationEligible == false
  and .host.version == "codex-cli 0.144.0"
  and .host.target == "x86_64-unknown-linux-musl"
  and .host.officialArchiveSha256 == "6b03d2d89910874fa5be27b617621d7638f906e891fd8cb40af3d2876a8a36fd"
  and .host.officialBinarySha256 == "901923c1808a151f6926d41d703c17ad48815662cefb1c8d832a052c44271429"
  and .host.officialVerified == false
' "${state}/state.json" >/dev/null || fail 'fixture state is not strictly non-qualifying'
pass

project="$(jq -er '.paths.project' "${state}/state.json")"
cat >"${codex_home}/config.toml" <<EOF
[projects."${project}"]
trust_level = "trusted"
EOF

"${fixture_producer}" begin --state "${state}" >/dev/null
jq -e '
  .phase == "ready"
  and .engine == {gateMode:"active",autopilotMode:"conservative"}
  and .config.approvalPolicy == "on-request"
  and .config.approvalsReviewer == "user"
  and .config.sandboxMode == "workspace-write"
  and .config.workspaceWriteNetworkAccess == false
  and .config.bypassHookTrust == false
  and .config.bypassApprovalsAndSandbox == false
' "${state}/state.json" >/dev/null || fail 'ready state omitted safe effective config'
pass

expect_failure 'normal Codex launch marker' \
  "${fixture_producer}" finalize --state "${state}" --out "${temporary}/before-launch"
expect_failure 'launcher accepts no arguments' "${state}/launch-codex.sh" unexpected
printf 'transient drift\n' >"${project}/transient-before-launch.txt"
expect_failure 'project inventory changed before launch' "${state}/launch-codex.sh"
rm "${project}/transient-before-launch.txt"
"${state}/launch-codex.sh" >/dev/null
jq -e \
  --arg home "${codex_home}" \
  --arg project "${project}" \
  --arg inventory "$(jq -er '.projectInventorySha256' "${state}/state.json")" '
    .schemaVersion == "brainmap-m8-codex-fia5-launch-v2"
    and .codexHome == $home
    and .projectInventorySha256 == $inventory
    and .argv[0:10] == [
      "--ask-for-approval","on-request","--sandbox","workspace-write",
      "-c","approvals_reviewer=\"user\"",
      "-c","sandbox_workspace_write.network_access=false","--cd",$project
    ]
    and .argv[10] == "--no-alt-screen"
    and (.argv | length) == 12
  ' "${state}/private/launch-marker.json" >/dev/null ||
  fail 'launcher did not persist the exact safe argv and CODEX_HOME'
pass

printf 'drift\n' >"${project}/unexpected.txt"
expect_failure 'project inventory changed during the Codex session' \
  "${fixture_producer}" finalize --state "${state}" --out "${temporary}/project-drift"
rm "${project}/unexpected.txt"
mkdir "${project}/unexpected-empty-directory"
expect_failure 'project inventory changed during the Codex session' \
  "${fixture_producer}" finalize --state "${state}" --out "${temporary}/directory-drift"
rmdir "${project}/unexpected-empty-directory"

BRAINMAP_GATE_MODE=shadow expect_failure 'cannot override Brainmap gate or autopilot mode' \
  "${fixture_producer}" finalize --state "${state}" --out "${temporary}/gate-override"
FIA5_FIXTURE_BAD_ENGINE=1 expect_failure 'active gate mode with conservative autopilot' \
  "${fixture_producer}" finalize --state "${state}" --out "${temporary}/bad-engine"
FIA5_FIXTURE_UNSAFE_APPROVAL=1 expect_failure 'effective config is unsafe' \
  "${fixture_producer}" finalize --state "${state}" --out "${temporary}/unsafe-approval"
FIA5_FIXTURE_UNSAFE_REVIEWER=1 expect_failure 'effective config is unsafe' \
  "${fixture_producer}" finalize --state "${state}" --out "${temporary}/unsafe-reviewer"
FIA5_FIXTURE_UNSAFE_SANDBOX=1 expect_failure 'effective config is unsafe' \
  "${fixture_producer}" finalize --state "${state}" --out "${temporary}/unsafe-sandbox"
FIA5_FIXTURE_MISSING_NETWORK=1 expect_failure 'effective config is unsafe' \
  "${fixture_producer}" finalize --state "${state}" --out "${temporary}/missing-network"
FIA5_FIXTURE_BYPASS_CONFIG=1 expect_failure 'effective config is unsafe' \
  "${fixture_producer}" finalize --state "${state}" --out "${temporary}/bypass-config"
FIA5_FIXTURE_UNTRUSTED=1 expect_failure 'persisted as trusted' \
  "${fixture_producer}" finalize --state "${state}" --out "${temporary}/untrusted"
FIA5_FIXTURE_EXTRA_HOOK=1 expect_failure 'persisted as trusted' \
  "${fixture_producer}" finalize --state "${state}" --out "${temporary}/extra-hook"
FIA5_FIXTURE_BAD_ORDER=1 expect_failure 'exact completed Brainmap MCP lifecycle' \
  "${fixture_producer}" finalize --state "${state}" --out "${temporary}/bad-order"
FIA5_FIXTURE_BAD_RUNTIME_ID=1 expect_failure 'exact completed Brainmap MCP lifecycle' \
  "${fixture_producer}" finalize --state "${state}" --out "${temporary}/bad-runtime-id"
FIA5_FIXTURE_BAD_SECOND_OUTCOME=1 expect_failure 'exact completed Brainmap MCP lifecycle' \
  "${fixture_producer}" finalize --state "${state}" --out "${temporary}/bad-second-outcome"
FIA5_FIXTURE_BAD_SECOND_RECORD=1 expect_failure 'exact completed Brainmap MCP lifecycle' \
  "${fixture_producer}" finalize --state "${state}" --out "${temporary}/bad-second-record"
FIA5_FIXTURE_DROPPED_REJECTED=1 expect_failure 'exact completed Brainmap MCP lifecycle' \
  "${fixture_producer}" finalize --state "${state}" --out "${temporary}/dropped-rejected"
FIA5_FIXTURE_SECOND_USER=1 expect_failure 'exact completed Brainmap MCP lifecycle' \
  "${fixture_producer}" finalize --state "${state}" --out "${temporary}/second-user"
FIA5_FIXTURE_SIDE_EFFECT_ITEM=1 expect_failure 'exact completed Brainmap MCP lifecycle' \
  "${fixture_producer}" finalize --state "${state}" --out "${temporary}/side-effect"
FIA5_FIXTURE_EXTRA_TOOL=1 expect_failure 'exact completed Brainmap MCP lifecycle' \
  "${fixture_producer}" finalize --state "${state}" --out "${temporary}/extra-tool"
rm -f "${state}/private/late-thread-list-count"
FIA5_FIXTURE_LATE_EXTRA_THREAD=1 expect_failure 'exactly one post-launch normal Codex CLI thread' \
  "${fixture_producer}" finalize --state "${state}" --out "${temporary}/late-extra-thread"
rm -f "${state}/private/late-thread-list-count"

ledger="$(jq -er '.paths.vault' "${state}/state.json")/90-calibration/decision-ledger.jsonl"
cp "${ledger}" "${temporary}/ledger.backup"
head -n -1 "${ledger}" >"${temporary}/ledger.bad"
tail -n 1 "${ledger}" | jq -c '.chosen = "biome"' >>"${temporary}/ledger.bad"
mv "${temporary}/ledger.bad" "${ledger}"
expect_failure 'ledger does not correlate' \
  "${fixture_producer}" finalize --state "${state}" --out "${temporary}/bad-ledger"
cp "${temporary}/ledger.backup" "${ledger}"
printf '{"kind":"unexpected","createdAt":"2026-07-10T17:00:06Z"}\n' >>"${ledger}"
expect_failure 'ledger does not correlate' \
  "${fixture_producer}" finalize --state "${state}" --out "${temporary}/extra-ledger-event"
cp "${temporary}/ledger.backup" "${ledger}"

expect_failure 'outside CODEX_HOME' \
  "${fixture_producer}" finalize --state "${state}" --out "${codex_home}/evidence"
lock_out="${temporary}/locked-evidence"
mkdir "${lock_out}.lock"
expect_failure 'publication lock already exists' \
  "${fixture_producer}" finalize --state "${state}" --out "${lock_out}"
rmdir "${lock_out}.lock"

"${fixture_producer}" finalize --state "${state}" --out "${out}" >/dev/null
[[ -d "${out}" && ! -L "${out}" ]] || fail 'evidence was not atomically published'
[[ ! -e "${out}.lock" && ! -L "${out}.lock" ]] || fail 'publication lock leaked'
pass
(cd "${out}" && sha256sum -c SHA256SUMS >/dev/null) || fail 'recursive checksums failed'
pass

brainmap_sha="$(sha256sum "${brainmap}" | cut -d ' ' -f 1)"
brainmapd_sha="$(sha256sum "${brainmapd}" | cut -d ' ' -f 1)"
jq -e \
  --arg commit "${candidate_commit}" \
  --arg brainmapSha "${brainmap_sha}" \
  --arg brainmapdSha "${brainmapd_sha}" '
  keys == [
    "adapter", "artifacts", "candidate", "completedAt", "mode", "privacy",
    "provenance", "qualificationEligible", "schemaVersion", "startedAt"
  ]
  and .schemaVersion == "brainmap-m8-host-v2"
  and .mode == "fixture"
  and .qualificationEligible == false
  and .candidate == {
    commit:$commit,brainmapSha256:$brainmapSha,brainmapdSha256:$brainmapdSha
  }
  and .adapter == {
    target:"codex",hostVersion:"codex-cli 0.144.0",launchMode:"normal",
    trustBypassUsed:false,persistedHookAccepted:true,projectTrusted:true
  }
  and .provenance.codexTarget == "x86_64-unknown-linux-musl"
  and .provenance.officialCodexArchiveSha256 == "6b03d2d89910874fa5be27b617621d7638f906e891fd8cb40af3d2876a8a36fd"
  and .provenance.officialCodexBinarySha256 == "901923c1808a151f6926d41d703c17ad48815662cefb1c8d832a052c44271429"
  and .provenance.officialCodexVerified == false
  and .artifacts.hostObservation.path == "host-observation.json"
  and .privacy == {
    rawPromptsRetained:false,secretsRetained:false,
    privatePathsRetained:false,syntheticInputsOnly:true
  }
' "${out}/manifest.json" >/dev/null || fail 'strict host-v2 manifest is invalid'
pass

host_observation_sha="$(sha256sum "${out}/host-observation.json" | cut -d ' ' -f 1)"
[[ "$(jq -er '.artifacts.hostObservation.sha256' "${out}/manifest.json")" == \
   "${host_observation_sha}" ]] || fail 'manifest host-observation digest is not bound'
pass

jq -e '
  .schemaVersion == "brainmap-m8-host-observation-v2"
  and .mode == "fixture"
  and .qualificationEligible == false
  and .officialCodex.version == "codex-cli 0.144.0"
  and .officialCodex.target == "x86_64-unknown-linux-musl"
  and .officialCodex.archiveVerified == false
  and .officialCodex.binaryVerified == false
  and .config.approvalPolicy == "on-request"
  and .config.approvalsReviewer == "user"
  and .config.sandboxMode == "workspace-write"
  and .config.workspaceWriteNetworkAccess == false
  and .config.bypassHookTrust == false
  and .config.bypassApprovalsAndSandbox == false
  and .config.gateMode == "active"
  and .config.autopilotMode == "conservative"
  and .launch.codexHomeBound == true
  and .hooks.trustedHookCount == 2
  and .hooks.executedHookGateCount >= 1
  and .calls.count == 7
  and .calls.order == [
    "brainmap_decision_gate","brainmap_record_decision",
    "brainmap_learn_feedback","brainmap_preview_update",
    "brainmap_apply_update","brainmap_decision_gate",
    "brainmap_record_decision"
  ]
  and .calls.first.outcome == "ask_user"
  and .calls.first.selectedOption == null
  and .calls.first.action == {chosen:"biome",wasAsked:true}
  and .calls.second.outcome == "proceed"
  and .calls.second.selectedOption == "prettier"
  and .calls.second.changed == true
  and .calls.second.action == {chosen:"prettier",wasAsked:false}
  and .calls.first.decisionId != .calls.second.decisionId
  and .ledger == {
    correlation:"complete",correlatedEventCount:5,postBoundaryEventCount:6
  }
  and .project.unchanged == true
' "${out}/host-observation.json" >/dev/null || fail 'strict host observation is invalid'
pass

jq -se '
  length == 12
  and [.[].sequence] == [1,2,3,4,5,6,7,8,9,10,11,12]
  and [.[].kind] == [
    "installer-dry-run","installed","doctor-healthy","host-launched",
    "initial-gate","initial-outcome-followed","initial-action-recorded",
    "feedback-created","preview-observed","update-approved",
    "changed-outcome-followed","changed-action-recorded"
  ]
  and (all(.[]; .success == true))
  and (.[4].decisionId | test("^dec_[0-9]{13}_[0-9a-f]{12}$"))
  and (.[4].decisionId as $first | all(.[4:10][]; .decisionId == $first))
  and (.[10].decisionId | test("^dec_[0-9]{13}_[0-9a-f]{12}$"))
  and .[10].decisionId != .[4].decisionId
  and (.[10].decisionId as $second | all(.[10:][]; .decisionId == $second))
  and (.[7].packetId as $packet | all(.[7:][]; .packetId == $packet))
  and .[10].changed == true
  and .[10].outcome == "proceed"
  and .[10].selectedOption == "prettier"
' "${out}/events.jsonl" >/dev/null || fail 'derived 12-event lifecycle is invalid'
pass

for private in "${state}" "${project}" "${codex_home}"; do
  grep -FR -- "${private}" "${out}" >/dev/null &&
    fail "evidence leaked private path ${private}"
done
pass
grep -ERi '"(prompt|messages|transcript|situation|options|toolarguments)"[[:space:]]*:' "${out}" >/dev/null &&
  fail 'evidence retained a raw prompt, transcript, or decision field'
pass

expect_failure 'evidence path already exists' \
  "${fixture_producer}" finalize --state "${state}" --out "${out}"

printf 'm8 Codex FIA-5 producer interface: %s checks passed\n' "${checks}"
