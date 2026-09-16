#!/bin/bash

# ==============================================================================
# Review API 测试脚本
# 可测试本地或远程服务器
# 用法: ./test_review_api.sh [local|remote|URL]
# ==============================================================================

set -e

# 颜色输出
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# 默认地址
LOCAL_URL="http://localhost:8080"
REMOTE_URL="http://123.57.182.243:8080"

# 解析参数
case "${1:-local}" in
    local)
        BASE_URL="$LOCAL_URL"
        ;;
    remote)
        BASE_URL="$REMOTE_URL"
        ;;
    *)
        BASE_URL="$1"
        ;;
esac

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}   Review API 测试${NC}"
echo -e "${BLUE}   目标: ${BASE_URL}${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

# 统计
PASSED=0
FAILED=0

# 测试函数
test_api() {
    local method="$1"
    local endpoint="$2"
    local data="$3"
    local expected_field="$4"
    local description="$5"
    
    echo -n "  测试: $description ... "
    
    if [ "$method" = "GET" ]; then
        response=$(curl -s -w "\n%{http_code}" "${BASE_URL}${endpoint}" 2>/dev/null || echo -e "\n000")
    else
        response=$(curl -s -w "\n%{http_code}" -X "$method" \
            -H "Content-Type: application/json" \
            -d "$data" \
            "${BASE_URL}${endpoint}" 2>/dev/null || echo -e "\n000")
    fi
    
    http_code=$(echo "$response" | tail -n1)
    body=$(echo "$response" | sed '$d')
    
    if [ "$http_code" = "200" ] || [ "$http_code" = "201" ]; then
        if [ -n "$expected_field" ]; then
            if echo "$body" | grep -q "\"$expected_field\""; then
                echo -e "${GREEN}✅ PASS${NC} (HTTP $http_code)"
                ((PASSED++))
                return 0
            else
                echo -e "${RED}❌ FAIL${NC} (missing field: $expected_field)"
                ((FAILED++))
                return 1
            fi
        else
            echo -e "${GREEN}✅ PASS${NC} (HTTP $http_code)"
            ((PASSED++))
            return 0
        fi
    elif [ "$http_code" = "000" ]; then
        echo -e "${RED}❌ FAIL${NC} (连接失败)"
        ((FAILED++))
        return 1
    else
        echo -e "${YELLOW}⚠️  WARN${NC} (HTTP $http_code)"
        ((FAILED++))
        return 1
    fi
}

# 保存创建的资源 ID
TASK_ID=""
RECORD_ID=""
COMMENT_ID=""

# ==================================================
# 1. 用户 API 测试
# ==================================================
echo -e "${YELLOW}📋 1. 用户 API${NC}"

test_api "GET" "/api/users" "" "success" "获取用户列表"
test_api "GET" "/api/users/me" "" "user" "获取当前用户"
test_api "GET" "/api/users/reviewers" "" "users" "获取审核人员"

echo ""

# ==================================================
# 2. 提资单 API 测试
# ==================================================
echo -e "${YELLOW}📋 2. 提资单 API${NC}"

# 创建提资单
echo -n "  测试: 创建提资单 ... "
response=$(curl -s -w "\n%{http_code}" -X POST \
    -H "Content-Type: application/json" \
    -d '{
        "title": "测试提资单-'$(date +%s)'",
        "modelName": "test-model",
        "reviewerId": "user-002",
        "priority": "medium",
        "components": [{"id": "c1", "name": "/TEST", "refNo": "123_456", "type": "pipe"}]
    }' \
    "${BASE_URL}/api/review/tasks" 2>/dev/null || echo -e "\n000")

http_code=$(echo "$response" | tail -n1)
body=$(echo "$response" | sed '$d')

if [ "$http_code" = "200" ]; then
    TASK_ID=$(echo "$body" | grep -o '"id":"[^"]*"' | head -1 | cut -d'"' -f4)
    if [ -n "$TASK_ID" ]; then
        echo -e "${GREEN}✅ PASS${NC} (ID: $TASK_ID)"
        ((PASSED++))
    else
        echo -e "${YELLOW}⚠️  PASS${NC} (无法提取 ID)"
        ((PASSED++))
    fi
else
    echo -e "${RED}❌ FAIL${NC} (HTTP $http_code)"
    ((FAILED++))
fi

test_api "GET" "/api/review/tasks" "" "tasks" "获取任务列表"

if [ -n "$TASK_ID" ]; then
    test_api "GET" "/api/review/tasks/$TASK_ID" "" "task" "获取任务详情"
    test_api "PATCH" "/api/review/tasks/$TASK_ID" '{"title":"更新后的标题"}' "success" "更新任务"
    test_api "POST" "/api/review/tasks/$TASK_ID/start-review" "" "success" "开始审核"
    test_api "GET" "/api/review/tasks/$TASK_ID/history" "" "history" "获取审核历史"
fi

echo ""

# ==================================================
# 3. 确认记录 API 测试
# ==================================================
echo -e "${YELLOW}📋 3. 确认记录 API${NC}"

if [ -n "$TASK_ID" ]; then
    # 创建确认记录
    echo -n "  测试: 创建确认记录 ... "
    response=$(curl -s -w "\n%{http_code}" -X POST \
        -H "Content-Type: application/json" \
        -d '{
            "taskId": "'$TASK_ID'",
            "type": "batch",
            "annotations": [{"id": "a1", "text": "测试批注"}],
            "cloudAnnotations": [],
            "rectAnnotations": [],
            "obbAnnotations": [],
            "measurements": [],
            "note": "测试备注"
        }' \
        "${BASE_URL}/api/review/records" 2>/dev/null || echo -e "\n000")
    
    http_code=$(echo "$response" | tail -n1)
    body=$(echo "$response" | sed '$d')
    
    if [ "$http_code" = "200" ]; then
        RECORD_ID=$(echo "$body" | grep -o '"id":"[^"]*"' | head -1 | cut -d'"' -f4)
        echo -e "${GREEN}✅ PASS${NC} (ID: $RECORD_ID)"
        ((PASSED++))
    else
        echo -e "${RED}❌ FAIL${NC} (HTTP $http_code)"
        ((FAILED++))
    fi
    
    test_api "GET" "/api/review/records/by-task/$TASK_ID" "" "records" "获取任务记录"
fi

echo ""

# ==================================================
# 4. 评论 API 测试
# ==================================================
echo -e "${YELLOW}📋 4. 评论 API${NC}"

# 创建评论
echo -n "  测试: 创建评论 ... "
response=$(curl -s -w "\n%{http_code}" -X POST \
    -H "Content-Type: application/json" \
    -d '{
        "annotationId": "anno-test-1",
        "annotationType": "text",
        "authorId": "user-001",
        "authorName": "测试用户",
        "authorRole": "designer",
        "content": "这是一条测试评论"
    }' \
    "${BASE_URL}/api/review/comments" 2>/dev/null || echo -e "\n000")

http_code=$(echo "$response" | tail -n1)
body=$(echo "$response" | sed '$d')

if [ "$http_code" = "200" ]; then
    COMMENT_ID=$(echo "$body" | grep -o '"id":"[^"]*"' | head -1 | cut -d'"' -f4)
    echo -e "${GREEN}✅ PASS${NC} (ID: $COMMENT_ID)"
    ((PASSED++))
else
    echo -e "${RED}❌ FAIL${NC} (HTTP $http_code)"
    ((FAILED++))
fi

test_api "GET" "/api/review/comments/by-annotation/anno-test-1?type=text" "" "comments" "获取评论"

echo ""

# ==================================================
# 5. 校审流程同步 API 测试
# ==================================================
echo -e "${YELLOW}📋 5. 校审流程同步 API${NC}"

test_api "POST" "/api/review/workflow/sync" '{
    "form_id": "test-form-123",
    "token": "test-token",
    "action": "active",
    "actor": {"id": "user-001", "name": "设计师小张", "roles": "sj"},
    "next_step": {"assignee_id": "user-002", "name": "校对员小李", "roles": "jd"},
    "comments": "请校对审核"
}' "code" "同步校审流程"

echo ""

# ==================================================
# 6. 清理测试数据
# ==================================================
echo -e "${YELLOW}📋 6. 清理测试数据${NC}"

if [ -n "$COMMENT_ID" ]; then
    test_api "DELETE" "/api/review/comments/item/$COMMENT_ID" "" "success" "删除评论"
fi

if [ -n "$RECORD_ID" ]; then
    test_api "DELETE" "/api/review/records/item/$RECORD_ID" "" "success" "删除记录"
fi

if [ -n "$TASK_ID" ]; then
    test_api "DELETE" "/api/review/tasks/$TASK_ID" "" "success" "删除任务"
fi

echo ""

# ==================================================
# 7. 汇总结果
# ==================================================
echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}   测试结果汇总${NC}"
echo -e "${BLUE}========================================${NC}"
echo -e "  通过: ${GREEN}$PASSED${NC}"
echo -e "  失败: ${RED}$FAILED${NC}"
TOTAL=$((PASSED + FAILED))
echo -e "  总计: $TOTAL"
echo ""

if [ $FAILED -eq 0 ]; then
    echo -e "${GREEN}🎉 全部测试通过！${NC}"
    exit 0
else
    echo -e "${YELLOW}⚠️ 部分测试失败，请检查服务器状态${NC}"
    exit 1
fi
