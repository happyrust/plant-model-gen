#!/usr/bin/env bash
# ============================================================================
# 校审工作流端到端测试脚本
# 覆盖：embed-url → create task → submit 流转 → workflow/sync → return → delete
# 说明：
#   - 默认按“生产鉴权开启”断言错误 token 返回 401
#   - 若联调环境 review_auth.enabled=false，可通过环境变量覆盖：
#       REVIEW_AUTH_ENABLED=false ./shells/test_workflow_e2e.sh
#     或直接指定：
#       EXPECTED_INVALID_TOKEN_STATUS=200 ./shells/test_workflow_e2e.sh
# ============================================================================
set -uo pipefail

SERVER="${SERVER_URL:-http://123.57.182.243:3100}"
PROJECT_ID="${PROJECT_ID:-AvevaMarineSample}"
REVIEW_AUTH_ENABLED="${REVIEW_AUTH_ENABLED:-auto}"
EXPECTED_INVALID_TOKEN_STATUS="${EXPECTED_INVALID_TOKEN_STATUS:-}"

if [ -z "$EXPECTED_INVALID_TOKEN_STATUS" ]; then
    case "${REVIEW_AUTH_ENABLED,,}" in
        true|1|yes|on)
            EXPECTED_INVALID_TOKEN_STATUS="401"
            ;;
        false|0|no|off)
            EXPECTED_INVALID_TOKEN_STATUS="200"
            ;;
        auto)
            EXPECTED_INVALID_TOKEN_STATUS="401"
            ;;
        *)
            echo "WARN: REVIEW_AUTH_ENABLED=$REVIEW_AUTH_ENABLED 非法，回退 EXPECTED_INVALID_TOKEN_STATUS=401"
            EXPECTED_INVALID_TOKEN_STATUS="401"
            ;;
    esac
fi

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
NC='\033[0m'

PASSED=0
FAILED=0
TOTAL=0

assert_eq() {
    TOTAL=$((TOTAL + 1))
    if [ "$2" = "$3" ]; then
        echo -e "  ${GREEN}✓${NC} $1 (=$2)"
        PASSED=$((PASSED + 1))
    else
        echo -e "  ${RED}✗${NC} $1 (expected=$2, got=$3)"
        FAILED=$((FAILED + 1))
    fi
}

assert_contains() {
    TOTAL=$((TOTAL + 1))
    if echo "$2" | grep -q "$3"; then
        echo -e "  ${GREEN}✓${NC} $1"
        PASSED=$((PASSED + 1))
    else
        echo -e "  ${RED}✗${NC} $1 (missing '$3')"
        FAILED=$((FAILED + 1))
    fi
}

assert_not_empty() {
    TOTAL=$((TOTAL + 1))
    if [ -n "$2" ] && [ "$2" != "null" ] && [ "$2" != "None" ] && [ "$2" != "" ]; then
        echo -e "  ${GREEN}✓${NC} $1 (non-empty)"
        PASSED=$((PASSED + 1))
    else
        echo -e "  ${RED}✗${NC} $1 (empty)"
        FAILED=$((FAILED + 1))
    fi
}

jf() {
    python3 -c "
import sys, json
d = json.load(sys.stdin)
path = sys.argv[1].split('.')
v = d
for k in path:
    if k == '': continue
    if isinstance(v, list): v = v[int(k)]
    elif isinstance(v, dict): v = v.get(k, '')
    else: v = ''; break
if v is None: v = ''
print(v)
" "$2" <<< "$1" 2>/dev/null || echo ""
}

jlen() {
    python3 -c "
import sys, json
d = json.load(sys.stdin)
path = sys.argv[1].split('.')
v = d
for k in path:
    if k == '': continue
    if isinstance(v, list): v = v[int(k)]
    elif isinstance(v, dict): v = v.get(k, [])
    else: v = []; break
print(len(v) if isinstance(v, list) else 0)
" "$2" <<< "$1" 2>/dev/null || echo "0"
}

post() {
    curl -s -X POST "${SERVER}${1}" -H "Content-Type: application/json" "${@:3}" -d "$2" 2>/dev/null
}

post_status() {
    curl -s -o /dev/null -w '%{http_code}' -X POST "${SERVER}${1}" -H "Content-Type: application/json" "${@:3}" -d "$2" 2>/dev/null
}

echo -e "${BLUE}╔══════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║          校审工作流 E2E 测试                             ║${NC}"
echo -e "${BLUE}║          ${SERVER}                  ║${NC}"
echo -e "${BLUE}╚══════════════════════════════════════════════════════════╝${NC}"
echo "  review_auth=$REVIEW_AUTH_ENABLED, expected_invalid_token_status=$EXPECTED_INVALID_TOKEN_STATUS"
echo ""

# ============================================================================
# 1. embed-url: 获取新 form_id + JWT
# ============================================================================
echo -e "${YELLOW}▸ 1. embed-url: 生成新 form_id${NC}"

EMBED=$(post "/api/review/embed-url" "{\"project_id\":\"${PROJECT_ID}\",\"user_id\":\"SJ\"}")
EMBED_CODE=$(jf "$EMBED" "code")
FORM_ID=$(jf "$EMBED" "data.query.form_id")
JWT_SJ=$(jf "$EMBED" "data.token")
URL=$(jf "$EMBED" "url")

assert_eq "embed-url code=200" "200" "$EMBED_CODE"
assert_not_empty "form_id 已生成" "$FORM_ID"
assert_not_empty "JWT token 已生成" "$JWT_SJ"
assert_contains "URL 包含 form_id" "$URL" "$FORM_ID"
assert_eq "embed form.exists=true" "True" "$(jf "$EMBED" "data.form.exists")"
assert_eq "embed form.status=blank" "blank" "$(jf "$EMBED" "data.form.status")"
assert_eq "embed form.task_created=false" "False" "$(jf "$EMBED" "data.form.task_created")"

LIN_TASK=$(jf "$EMBED" "data.lineage.task_id")
assert_eq "lineage 无已有 task" "" "$LIN_TASK"

echo "  → form_id=$FORM_ID"
echo ""

# ============================================================================
# 2. workflow/sync query: 空 form 返回空数据
# ============================================================================
echo -e "${YELLOW}▸ 2. workflow/sync query: 新 form 无关联数据${NC}"

SYNC=$(post "/api/review/workflow/sync" "{
    \"form_id\":\"${FORM_ID}\",\"token\":\"${JWT_SJ}\",
    \"action\":\"query\",\"actor\":{\"id\":\"SJ\",\"name\":\"设计\",\"roles\":\"sj\"}}")
SYNC_CODE=$(jf "$SYNC" "code")
MODELS_N=$(jlen "$SYNC" "data.models")

assert_eq "sync code=200" "200" "$SYNC_CODE"
assert_eq "form_exists=true" "True" "$(jf "$SYNC" "data.form_exists")"
assert_eq "form_status=blank" "blank" "$(jf "$SYNC" "data.form_status")"
assert_eq "task_created=false" "False" "$(jf "$SYNC" "data.task_created")"
assert_eq "无关联模型" "0" "$MODELS_N"
echo ""

# ============================================================================
# 3. Token 校验: 错误 token 状态码断言（可配置）
# ============================================================================
echo -e "${YELLOW}▸ 3. Token 校验: 错误 token → ${EXPECTED_INVALID_TOKEN_STATUS}${NC}"

BAD_STATUS=$(post_status "/api/review/workflow/sync" "{
    \"form_id\":\"${FORM_ID}\",\"token\":\"invalid-token\",
    \"action\":\"query\",\"actor\":{\"id\":\"SJ\",\"name\":\"设计\",\"roles\":\"sj\"}}")
assert_eq "错误 token → ${EXPECTED_INVALID_TOKEN_STATUS}" "${EXPECTED_INVALID_TOKEN_STATUS}" "$BAD_STATUS"
echo ""

# ============================================================================
# 4. create task: 创建提资单并关联模型
# ============================================================================
echo -e "${YELLOW}▸ 4. 创建提资单 (SJ)${NC}"

CREATE=$(post "/api/review/tasks" "{
    \"title\":\"E2E-管道校审-$(date +%H%M%S)\",\"description\":\"E2E自动测试\",
    \"modelName\":\"${PROJECT_ID}\",\"checkerId\":\"JH\",\"approverId\":\"SH\",\"reviewerId\":\"JH\",
    \"formId\":\"${FORM_ID}\",\"priority\":\"medium\",
    \"components\":[
        {\"id\":\"c1\",\"refNo\":\"24381_145018\",\"name\":\"管道A\",\"type\":\"PIPE\"},
        {\"id\":\"c2\",\"refNo\":\"24381_145020\",\"name\":\"阀门B\",\"type\":\"VALVE\"},
        {\"id\":\"c3\",\"refNo\":\"24381_145022\",\"name\":\"支撑C\",\"type\":\"STRU\"}]}" \
    -H "Authorization: Bearer $JWT_SJ")

CREATE_OK=$(jf "$CREATE" "success")
TASK_ID=$(jf "$CREATE" "task.id")
TASK_STATUS=$(jf "$CREATE" "task.status")
TASK_NODE=$(jf "$CREATE" "task.currentNode")
TASK_FORM=$(jf "$CREATE" "task.formId")

assert_eq "创建成功" "True" "$CREATE_OK"
assert_not_empty "task_id" "$TASK_ID"
assert_eq "状态=draft" "draft" "$TASK_STATUS"
assert_eq "节点=sj" "sj" "$TASK_NODE"
assert_eq "form_id 一致" "$FORM_ID" "$TASK_FORM"
echo "  → task_id=$TASK_ID"
echo ""

# ============================================================================
# 5. workflow/sync query: 创建后有 models
# ============================================================================
echo -e "${YELLOW}▸ 5. workflow/sync query: 创建后有模型数据${NC}"

SYNC2=$(post "/api/review/workflow/sync" "{
    \"form_id\":\"${FORM_ID}\",\"token\":\"${JWT_SJ}\",
    \"action\":\"query\",\"actor\":{\"id\":\"SJ\",\"name\":\"设计\",\"roles\":\"sj\"}}")
MODELS_N2=$(jlen "$SYNC2" "data.models")
MODELS_STR=$(jf "$SYNC2" "data.models")

assert_eq "关联 3 个模型" "3" "$MODELS_N2"
assert_eq "form_status=draft" "draft" "$(jf "$SYNC2" "data.form_status")"
assert_eq "task_created=true" "True" "$(jf "$SYNC2" "data.task_created")"
assert_contains "包含 145018" "$MODELS_STR" "24381_145018"
assert_contains "包含 145020" "$MODELS_STR" "24381_145020"
echo ""

# ============================================================================
# 6. embed-url lineage: 应包含 task 信息
# ============================================================================
echo -e "${YELLOW}▸ 6. embed-url lineage: task 已关联${NC}"

EMBED2=$(post "/api/review/embed-url" "{\"project_id\":\"${PROJECT_ID}\",\"user_id\":\"SJ\",\"form_id\":\"${FORM_ID}\"}")
LIN2_TASK=$(jf "$EMBED2" "data.lineage.task_id")
LIN2_NODE=$(jf "$EMBED2" "data.lineage.current_node")
LIN2_STATUS=$(jf "$EMBED2" "data.lineage.status")

assert_eq "lineage task_id" "$TASK_ID" "$LIN2_TASK"
assert_eq "lineage node=sj" "sj" "$LIN2_NODE"
assert_eq "lineage status=draft" "draft" "$LIN2_STATUS"
assert_eq "embed2 form.status=draft" "draft" "$(jf "$EMBED2" "data.form.status")"
assert_eq "embed2 form.task_created=true" "True" "$(jf "$EMBED2" "data.form.task_created")"
echo ""

# ============================================================================
# 7. SJ → JH: 编制提交到校对
# ============================================================================
echo -e "${YELLOW}▸ 7. SJ → JH: 编制提交到校对${NC}"

SUBMIT1=$(post "/api/review/tasks/${TASK_ID}/submit" '{"comment":"设计完成"}' -H "Authorization: Bearer $JWT_SJ")
SUBMIT1_OK=$(jf "$SUBMIT1" "success")
assert_eq "SJ submit 成功" "True" "$SUBMIT1_OK"
assert_contains "消息含编制" "$SUBMIT1" "编制"
echo ""

# ============================================================================
# 8. PMS workflow/sync active: 平台 comments 不再由模型中心持久化/回传
# ============================================================================
echo -e "${YELLOW}▸ 8. PMS workflow/sync active：平台 comments 不落模型中心${NC}"

SYNC3=$(post "/api/review/workflow/sync" "{
    \"form_id\":\"${FORM_ID}\",\"token\":\"${JWT_SJ}\",
    \"action\":\"active\",
    \"actor\":{\"id\":\"SJ\",\"name\":\"设计\",\"roles\":\"sj\"},
    \"next_step\":{\"assignee_id\":\"JH\",\"name\":\"校核\",\"roles\":\"jd\"},
    \"comments\":\"E2E-设计完成\"}")
SYNC3_CODE=$(jf "$SYNC3" "code")
OP_N1=$(jlen "$SYNC3" "data.opinions")

assert_eq "sync code=200" "200" "$SYNC3_CODE"
assert_eq "opinions 不再回传" "0" "$OP_N1"
echo ""

# ============================================================================
# 9. JH → SH: 校对提交到审核
# ============================================================================
echo -e "${YELLOW}▸ 9. JH → SH: 校对提交到审核${NC}"

JH_EMBED=$(post "/api/review/embed-url" "{\"project_id\":\"${PROJECT_ID}\",\"user_id\":\"JH\",\"form_id\":\"${FORM_ID}\"}")
JWT_JH=$(jf "$JH_EMBED" "data.token")

SUBMIT2=$(post "/api/review/tasks/${TASK_ID}/submit" '{"comment":"校对通过"}' -H "Authorization: Bearer $JWT_JH")
assert_eq "JH submit 成功" "True" "$(jf "$SUBMIT2" "success")"

SYNC4=$(post "/api/review/workflow/sync" "{
    \"form_id\":\"${FORM_ID}\",\"token\":\"${JWT_JH}\",
    \"action\":\"agree\",
    \"actor\":{\"id\":\"JH\",\"name\":\"校核\",\"roles\":\"jd\"},
    \"next_step\":{\"assignee_id\":\"SH\",\"name\":\"审核\",\"roles\":\"sh\"},
    \"comments\":\"E2E-校对通过\"}")
OP_N2=$(jlen "$SYNC4" "data.opinions")
assert_eq "agree 后仍不回传 opinions" "0" "$OP_N2"
echo ""

# ============================================================================
# 10. SH → PZ: 审核提交到批准
# ============================================================================
echo -e "${YELLOW}▸ 10. SH → PZ: 审核提交到批准${NC}"

SH_EMBED=$(post "/api/review/embed-url" "{\"project_id\":\"${PROJECT_ID}\",\"user_id\":\"SH\",\"form_id\":\"${FORM_ID}\"}")
JWT_SH=$(jf "$SH_EMBED" "data.token")

SUBMIT3=$(post "/api/review/tasks/${TASK_ID}/submit" '{"comment":"审核通过"}' -H "Authorization: Bearer $JWT_SH")
assert_eq "SH submit 成功" "True" "$(jf "$SUBMIT3" "success")"

SYNC5=$(post "/api/review/workflow/sync" "{
    \"form_id\":\"${FORM_ID}\",\"token\":\"${JWT_SH}\",
    \"action\":\"agree\",
    \"actor\":{\"id\":\"SH\",\"name\":\"审核\",\"roles\":\"sh\"},
    \"comments\":\"E2E-审核通过\"}")
OP_N3=$(jlen "$SYNC5" "data.opinions")
assert_eq "审核后仍不回传 opinions" "0" "$OP_N3"
echo ""

# ============================================================================
# 11. 验证最终 lineage
# ============================================================================
echo -e "${YELLOW}▸ 11. 最终 lineage 验证${NC}"

FINAL=$(post "/api/review/embed-url" "{\"project_id\":\"${PROJECT_ID}\",\"user_id\":\"SJ\",\"form_id\":\"${FORM_ID}\"}")
FINAL_NODE=$(jf "$FINAL" "data.lineage.current_node")
FINAL_STATUS=$(jf "$FINAL" "data.lineage.status")

assert_eq "最终节点=pz" "pz" "$FINAL_NODE"
assert_eq "最终状态=in_review" "in_review" "$FINAL_STATUS"
echo ""

# ============================================================================
# 12. 驳回测试: PZ 驳回到 SJ
# ============================================================================
echo -e "${YELLOW}▸ 12. 驳回测试: PZ → SJ${NC}"

RETURN=$(post "/api/review/tasks/${TASK_ID}/return" \
    '{"targetNode":"sj","reason":"请修改管道走向","comment":"发现问题"}' \
    -H "Authorization: Bearer $JWT_SH")
assert_eq "驳回成功" "True" "$(jf "$RETURN" "success")"

AFTER=$(post "/api/review/embed-url" "{\"project_id\":\"${PROJECT_ID}\",\"user_id\":\"SJ\",\"form_id\":\"${FORM_ID}\"}")
assert_eq "驳回后节点=sj" "sj" "$(jf "$AFTER" "data.lineage.current_node")"

SYNC_RETURN=$(post "/api/review/workflow/sync" "{
    \"form_id\":\"${FORM_ID}\",\"token\":\"${JWT_SH}\",
    \"action\":\"return\",
    \"actor\":{\"id\":\"SH\",\"name\":\"审核\",\"roles\":\"sh\"},
    \"comments\":\"E2E-驳回\"}")
SYNC_RETURN_CODE=$(jf "$SYNC_RETURN" "code")
OP_N4=$(jlen "$SYNC_RETURN" "data.opinions")

assert_eq "驳回 sync code=200" "200" "$SYNC_RETURN_CODE"
assert_eq "4条意见(含驳回)" "4" "$OP_N4"
echo ""

# ============================================================================
# 13. 清理: 删除测试数据
# ============================================================================
echo -e "${YELLOW}▸ 13. 清理测试数据${NC}"

DEL=$(post "/api/review/delete" "{\"form_ids\":[\"${FORM_ID}\"],\"operator_id\":\"SJ\",\"token\":\"${JWT_SJ}\"}")
DEL_CODE=$(jf "$DEL" "code")
assert_eq "删除 code=200" "200" "$DEL_CODE"

VERIFY=$(post "/api/review/workflow/sync" "{
    \"form_id\":\"${FORM_ID}\",\"token\":\"${JWT_SJ}\",
    \"action\":\"query\",\"actor\":{\"id\":\"SJ\",\"name\":\"设计\",\"roles\":\"sj\"}}")
VERIFY_N=$(jlen "$VERIFY" "data.models")
# 当前删除语义为软删：主单/任务标记 deleted，不物理删除 review_form_model 关联。
# 因此删除后模型列表仍保留，数量应与删除前一致。
assert_eq "删除后模型仍保留(软删)" "$MODELS_N2" "$VERIFY_N"
assert_eq "删除后 form_exists=true" "True" "$(jf "$VERIFY" "data.form_exists")"
assert_eq "删除后 form_status=deleted" "deleted" "$(jf "$VERIFY" "data.form_status")"
assert_eq "删除后 task_created=false" "False" "$(jf "$VERIFY" "data.task_created")"
echo ""

# ============================================================================
# 汇总
# ============================================================================
echo -e "${BLUE}╔══════════════════════════════════════════════════════════╗${NC}"
if [ $FAILED -eq 0 ]; then
    echo -e "${BLUE}║  ${GREEN}ALL PASSED${NC}  ${PASSED}/${TOTAL} assertions"
else
    echo -e "${BLUE}║  ${RED}${FAILED} FAILED${NC}  ${PASSED}/${TOTAL} assertions"
fi
echo -e "  form_id: ${FORM_ID}"
echo -e "  task_id: ${TASK_ID:-N/A}"
echo -e "${BLUE}╚══════════════════════════════════════════════════════════╝${NC}"

exit $FAILED
