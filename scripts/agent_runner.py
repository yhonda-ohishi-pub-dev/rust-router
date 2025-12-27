#!/usr/bin/env python3
"""
Agent Runner - 自律的なエージェントチェーン実行

機能:
- plan.md から詳細タスクを生成
- コンテキスト使用量 50% で自動引き継ぎ
- HANDOVER.md で次のエージェントに引き継ぎ
- agent_log.txt にトークン使用量を記録
- Main には最終結果のみ通知
"""

import asyncio
import json
import os
import sys
from datetime import datetime
from pathlib import Path
from dataclasses import dataclass, field
from typing import Optional, AsyncIterator

from claude_code_sdk import (
    query,
    ClaudeCodeOptions,
    AssistantMessage,
    ResultMessage,
    SystemMessage,
    TextBlock,
    ToolUseBlock,
)


# 設定
CONTEXT_WINDOW = 200_000  # Claude のコンテキストウィンドウ
THRESHOLD = 0.80  # 80%（50%だと早すぎる）
LOG_FILE = "agent_log.txt"
HANDOVER_FILE = "HANDOVER.md"
PLAN_FILE = "plan.md"

# 手動タスクを示すキーワード（これらを含むタスクは自動スキップ候補）
MANUAL_TASK_KEYWORDS = [
    "手動",
    "ブラウザ",
    "フロントエンド",
    "手動テスト",
    "外部サービス",
    "保留",
    "将来タスク",
    "manual",
    "browser",
    "frontend",
]

# ファイルロック・致命的エラーを示すパターン（より厳密に）
# 単語だけでなく、エラー文脈を含むパターンで検出
FATAL_ERROR_PATTERNS = [
    "error: cannot remove",
    "error.*being used by another process",
    "error.*permission denied",
    "permissionerror:",
    "error.*access is denied",
    "failed.*locked",
    "エラー.*ロック",
    "失敗.*ロック",
    "error.*locked",
    "cannot write.*locked",
    "cannot delete.*locked",
]


@dataclass
class UsageStats:
    """トークン使用量の統計"""
    input_tokens: int = 0
    output_tokens: int = 0
    cache_read_tokens: int = 0
    cache_creation_tokens: int = 0
    total_cost_usd: float = 0.0

    @property
    def total_tokens(self) -> int:
        return self.input_tokens + self.output_tokens

    @property
    def usage_percent(self) -> float:
        return self.input_tokens / CONTEXT_WINDOW

    def update(self, usage: dict, cost: float = 0.0):
        """使用量を更新"""
        self.input_tokens += usage.get('input_tokens', 0) or 0
        self.output_tokens += usage.get('output_tokens', 0) or 0
        self.cache_read_tokens += usage.get('cache_read_input_tokens', 0) or 0
        self.cache_creation_tokens += usage.get('cache_creation_input_tokens', 0) or 0
        self.total_cost_usd += cost or 0.0


@dataclass
class AgentContext:
    """エージェントの実行コンテキスト"""
    agent_number: int
    tasks: list[str]
    completed_tasks: list[str] = field(default_factory=list)
    current_task_index: int = 0
    work_dir: str = "."
    stats: UsageStats = field(default_factory=UsageStats)

    @property
    def current_task(self) -> Optional[str]:
        if self.current_task_index < len(self.tasks):
            return self.tasks[self.current_task_index]
        return None

    @property
    def remaining_tasks(self) -> list[str]:
        return self.tasks[self.current_task_index:]


class AgentLogger:
    """ログファイル出力"""

    def __init__(self, log_path: Path):
        self.log_path = log_path
        self.log_path.parent.mkdir(parents=True, exist_ok=True)

    def log(self, message: str, level: str = "INFO"):
        timestamp = datetime.now().isoformat()
        line = f"[{timestamp}] [{level}] {message}\n"
        with open(self.log_path, 'a', encoding='utf-8') as f:
            f.write(line)

    def log_separator(self, char: str = "=", length: int = 70):
        """区切り線を出力"""
        with open(self.log_path, 'a', encoding='utf-8') as f:
            f.write(f"\n{char * length}\n\n")

    def log_task_list(self, tasks: list[str], title: str = "TASK LIST"):
        """タスク一覧を出力"""
        self.log_separator()
        self.log(f"{'='*20} {title} {'='*20}")
        self.log(f"Total: {len(tasks)} tasks")
        self.log("")
        for i, task in enumerate(tasks, 1):
            status = "[ ]"
            self.log(f"  {i:2d}. {status} {task}")
        self.log_separator("-", 50)

    def log_task_status(self, ctx: 'AgentContext'):
        """現在のタスク状態を出力"""
        self.log("")
        self.log(f"--- TASK STATUS (Agent #{ctx.agent_number}) ---")
        for i, task in enumerate(ctx.tasks):
            if i < len(ctx.completed_tasks):
                status = "[x]"
                marker = "DONE"
            elif i == ctx.current_task_index:
                status = "[>]"
                marker = ">>> CURRENT <<<"
            else:
                status = "[ ]"
                marker = ""
            self.log(f"  {i+1:2d}. {status} {task} {marker}")
        self.log("")

    def log_task_start(self, ctx: 'AgentContext', task: str):
        """タスク開始をログ"""
        self.log_separator("-", 50)
        self.log(f"TASK START: [{ctx.current_task_index + 1}/{len(ctx.tasks)}]", "TASK")
        self.log(f"  Task: {task}", "TASK")
        self.log(f"  Agent: #{ctx.agent_number}", "TASK")
        self.log_task_status(ctx)

    def log_task_complete(self, ctx: 'AgentContext', task: str):
        """タスク完了をログ"""
        self.log(f"TASK COMPLETE: {task}", "TASK")
        self.log_usage(ctx)

    def log_usage(self, ctx: 'AgentContext'):
        """使用量をログ"""
        bar_length = 30
        filled = int(bar_length * ctx.stats.usage_percent)
        bar = "█" * filled + "░" * (bar_length - filled)

        self.log(
            f"  Usage: [{bar}] {ctx.stats.usage_percent*100:5.1f}% | "
            f"In: {ctx.stats.input_tokens:,} | "
            f"Out: {ctx.stats.output_tokens:,} | "
            f"Cost: ${ctx.stats.total_cost_usd:.4f}",
            "USAGE"
        )

    def log_agent_start(self, ctx: 'AgentContext'):
        """エージェント開始をログ"""
        self.log_separator("=")
        self.log(f"AGENT #{ctx.agent_number} STARTED", "AGENT")
        self.log(f"  Work Dir: {ctx.work_dir}")
        self.log(f"  Tasks: {len(ctx.tasks)} total, {len(ctx.remaining_tasks)} remaining")
        self.log(f"  Threshold: {THRESHOLD*100:.0f}%")
        self.log_task_status(ctx)

    def log_agent_end(self, ctx: 'AgentContext', reason: str):
        """エージェント終了をログ"""
        self.log_separator("-", 50)
        self.log(f"AGENT #{ctx.agent_number} ENDED", "AGENT")
        self.log(f"  Reason: {reason}")
        self.log(f"  Completed: {len(ctx.completed_tasks)}/{len(ctx.tasks)} tasks")
        self.log_usage(ctx)
        self.log_task_status(ctx)

    def log_handover(self, from_agent: int, to_agent: int, reason: str):
        """引き継ぎをログ"""
        self.log_separator("*", 50)
        self.log(f"HANDOVER: Agent #{from_agent} -> Agent #{to_agent}", "HANDOVER")
        self.log(f"  Reason: {reason}", "HANDOVER")
        self.log_separator("*", 50)

    def log_chain_start(self, tasks: list[str]):
        """チェーン開始をログ"""
        self.log_separator("=")
        self.log("AGENT CHAIN STARTED", "CHAIN")
        self.log(f"  Started: {datetime.now().isoformat()}")
        self.log_task_list(tasks)

    def log_chain_end(self, success: bool, total_agents: int):
        """チェーン終了をログ"""
        self.log_separator("=")
        status = "SUCCESS" if success else "FAILED"
        self.log(f"AGENT CHAIN {status}", "CHAIN")
        self.log(f"  Total Agents: {total_agents}")
        self.log(f"  Ended: {datetime.now().isoformat()}")
        self.log_separator("=")


def create_handover(ctx: AgentContext, reason: str) -> str:
    """引継ぎ書を生成"""
    content = f"""# Agent Handover Document

## From Agent #{ctx.agent_number}
Generated: {datetime.now().isoformat()}

## Reason for Handover
{reason}

## Usage Statistics
- Input Tokens: {ctx.stats.input_tokens:,}
- Output Tokens: {ctx.stats.output_tokens:,}
- Context Usage: {ctx.stats.usage_percent*100:.1f}%
- Total Cost: ${ctx.stats.total_cost_usd:.4f}

## Completed Tasks
{chr(10).join(f'- [x] {task}' for task in ctx.completed_tasks) or '(none)'}

## Remaining Tasks
{chr(10).join(f'- [ ] {task}' for task in ctx.remaining_tasks)}

## Current Task (In Progress)
{ctx.current_task or '(none)'}

## Work Directory
{ctx.work_dir}

## Instructions for Next Agent
1. Read this handover document
2. Continue from the current task
3. Complete remaining tasks in order
4. Create handover if context exceeds 50%
"""
    return content


def is_manual_task(task: str) -> bool:
    """タスクが手動タスクかどうかを判定"""
    task_lower = task.lower()
    return any(keyword.lower() in task_lower for keyword in MANUAL_TASK_KEYWORDS)


def detect_fatal_error(text: str) -> Optional[str]:
    """致命的エラー（ロック等）を検出（正規表現パターンで厳密に）"""
    import re
    text_lower = text.lower()
    for pattern in FATAL_ERROR_PATTERNS:
        if re.search(pattern, text_lower):
            return pattern
    return None


def parse_plan(plan_content: str, skip_manual: bool = False) -> tuple[list[str], list[str]]:
    """
    plan.md からタスクを抽出

    Args:
        plan_content: plan.md の内容
        skip_manual: True の場合、手動タスクをスキップリストに分離

    Returns:
        (tasks, manual_tasks): 実行するタスクと手動タスクのリスト
    """
    tasks = []
    manual_tasks = []

    for line in plan_content.split('\n'):
        line = line.strip()

        # チェックリスト項目を検出
        if line.startswith('- [ ]'):
            task = line[5:].strip()
            if task:
                if skip_manual and is_manual_task(task):
                    manual_tasks.append(task)
                else:
                    tasks.append(task)
        elif line.startswith('- [x]') or line.startswith('- [X]'):
            # 完了済みはスキップ
            continue

    return tasks, manual_tasks


def build_agent_prompt(ctx: AgentContext, handover_content: Optional[str] = None) -> str:
    """エージェント用のプロンプトを構築"""

    common_instructions = f"""
## 重要な指示

1. **タスク完了時は必ず plan.md を更新**してください
   - `- [ ] タスク名` を `- [x] タスク名` に変更
   - これにより進捗が正しく追跡されます

2. 各タスクを順番に実行してください

3. タスクが現在の環境では実行不可能な場合（外部サービス依存、手動作業が必要など）:
   - plan.md にコメントを追加して理由を説明
   - 可能な部分だけ完了としてマーク
   - 次のタスクに進む

4. 作業ディレクトリ: {ctx.work_dir}

5. すべてのタスクを完了するか、これ以上進められなくなるまで作業を続けてください

6. **致命的エラー発生時は即座に中断**してください:
   - ファイルがロックされている（別プロセスで使用中）
   - Permission denied / Access is denied
   - ビルドが exe ロックで失敗
   これらのエラーが発生したら、それ以上作業を続けず、エラー内容を報告して終了してください。
   引き継ぎは不要です。ユーザーがロックを解除してから再実行する必要があります。
"""

    if handover_content:
        # 引き継ぎモード
        prompt = f"""あなたは Agent #{ctx.agent_number} です。前のエージェントから引き継ぎました。

## 引継ぎ書
{handover_content}
{common_instructions}

引継ぎ書を確認し、残りのタスクから作業を開始してください。
"""
    else:
        # 新規開始モード
        tasks_list = '\n'.join(f'{i+1}. [ ] {task}' for i, task in enumerate(ctx.tasks))
        prompt = f"""あなたは Agent #{ctx.agent_number} です。

## 未完了タスク一覧（plan.md より）
{tasks_list}
{common_instructions}

最初のタスクから開始してください。
"""

    return prompt


def check_plan_completion(plan_path: Path, skip_manual: bool = False) -> tuple[list[str], list[str], list[str]]:
    """
    plan.md を読み直して完了/未完了タスクを取得

    Args:
        plan_path: plan.md のパス
        skip_manual: True の場合、手動タスクをスキップ

    Returns:
        (completed_tasks, remaining_tasks, skipped_manual_tasks)
    """
    content = plan_path.read_text(encoding='utf-8')
    completed = []
    remaining = []
    skipped = []

    for line in content.split('\n'):
        line_stripped = line.strip()
        if line_stripped.startswith('- [x]') or line_stripped.startswith('- [X]'):
            task = line_stripped[5:].strip()
            if task:
                completed.append(task)
        elif line_stripped.startswith('- [ ]'):
            task = line_stripped[5:].strip()
            if task:
                if skip_manual and is_manual_task(task):
                    skipped.append(task)
                else:
                    remaining.append(task)

    return completed, remaining, skipped


async def run_agent(
    ctx: AgentContext,
    logger: AgentLogger,
    plan_path: Path,
    handover_content: Optional[str] = None
) -> tuple[bool, Optional[str]]:
    """
    単一エージェントを実行

    Returns:
        (completed, handover_content):
        - completed=True: 全タスク完了
        - completed=False + handover: 引き継ぎが必要
    """

    prompt = build_agent_prompt(ctx, handover_content)

    # エージェント開始ログ
    logger.log_agent_start(ctx)

    # 現在のタスク開始ログ
    if ctx.current_task:
        logger.log_task_start(ctx, ctx.current_task)

    options = ClaudeCodeOptions(
        max_turns=100,  # 十分な回数を許可
        permission_mode='bypassPermissions',  # 全ツール自動承認
    )

    try:
        async for message in query(prompt=prompt, options=options):
            # isinstance() でメッセージタイプを判定
            if isinstance(message, ResultMessage):
                # ResultMessage: 実行結果、usage情報を含む
                logger.log(f"[RESULT] subtype={message.subtype}, duration={message.duration_ms}ms", "DEBUG")

                # usage を取得
                if message.usage:
                    logger.log(f"[RESULT] usage: {message.usage}", "DEBUG")
                    ctx.stats.update(message.usage, message.total_cost_usd or 0.0)
                    logger.log(
                        f"[RESULT] Updated stats: in={ctx.stats.input_tokens}, "
                        f"out={ctx.stats.output_tokens}, pct={ctx.stats.usage_percent*100:.1f}%",
                        "DEBUG"
                    )

                # 閾値チェック
                if ctx.stats.usage_percent >= THRESHOLD:
                    reason = f"Context usage exceeded {THRESHOLD*100:.0f}% ({ctx.stats.usage_percent*100:.1f}%)"
                    logger.log(reason, "WARNING")
                    logger.log_agent_end(ctx, reason)

                    handover = create_handover(ctx, reason)
                    return False, handover

                # 結果ログ
                logger.log(f"Agent finished: {message.subtype}", "RESULT")
                if message.result:
                    logger.log(f"  Result: {message.result[:300]}...", "RESULT")
                logger.log_usage(ctx)

            elif isinstance(message, AssistantMessage):
                # AssistantMessage: Claude からの応答
                for block in message.content:
                    if isinstance(block, TextBlock):
                        short_text = block.text[:150].replace('\n', ' ')
                        logger.log(f"[Agent] {short_text}...")

                        # 致命的エラー検出
                        fatal_keyword = detect_fatal_error(block.text)
                        if fatal_keyword:
                            reason = f"Fatal error detected: '{fatal_keyword}' - File may be locked"
                            logger.log(reason, "FATAL")
                            logger.log_agent_end(ctx, reason)
                            # 引き継ぎなしで終了（ロック解除が必要）
                            return False, None

                    elif isinstance(block, ToolUseBlock):
                        logger.log(f"[Tool] {block.name}", "DEBUG")

            elif isinstance(message, SystemMessage):
                # SystemMessage: システムメッセージ（init等）
                logger.log(f"[System] subtype={message.subtype}", "DEBUG")

    except Exception as e:
        logger.log(f"Agent error: {e}", "ERROR")
        logger.log_agent_end(ctx, f"Error: {e}")
        handover = create_handover(ctx, f"Error: {e}")
        return False, handover

    # plan.md を読み直して実際の完了状態をチェック（skip_manual は run_agent_chain から渡される）
    completed, remaining, skipped = check_plan_completion(plan_path, skip_manual=True)

    logger.log(f"Plan check: {len(completed)} completed, {len(remaining)} remaining, {len(skipped)} skipped (manual)", "CHECK")

    if not remaining:
        if skipped:
            logger.log(f"Automated tasks completed. {len(skipped)} manual tasks remain:", "INFO")
            for task in skipped:
                logger.log(f"  - [MANUAL] {task}", "INFO")
        logger.log_agent_end(ctx, "All automated tasks completed (verified from plan.md)")
        return True, None
    else:
        # まだ残タスクがある - 引き継ぎ
        reason = f"Agent session ended with {len(remaining)} tasks remaining"
        logger.log(reason, "WARNING")

        # コンテキストを更新して引き継ぎ書作成
        ctx.completed_tasks = completed
        ctx.tasks = completed + remaining
        ctx.current_task_index = len(completed)

        logger.log_agent_end(ctx, reason)
        handover = create_handover(ctx, reason)
        return False, handover


async def run_agent_chain(
    plan_path: Path,
    work_dir: Path,
    log_path: Path,
    handover_path: Path,
    max_agents: int = 10,
    skip_manual: bool = True
) -> bool:
    """
    エージェントチェーンを実行

    Args:
        plan_path: plan.md のパス
        work_dir: 作業ディレクトリ
        log_path: ログファイルのパス
        handover_path: 引継ぎ書のパス
        max_agents: 最大エージェント数
        skip_manual: True の場合、手動タスクをスキップ
    """
    logger = AgentLogger(log_path)

    # plan.md を読み込み
    plan_content = plan_path.read_text(encoding='utf-8')
    tasks, manual_tasks = parse_plan(plan_content, skip_manual=skip_manual)

    if not tasks:
        if manual_tasks:
            logger.log(f"No automated tasks found. {len(manual_tasks)} manual tasks skipped:", "INFO")
            for task in manual_tasks:
                logger.log(f"  - [MANUAL] {task}", "INFO")
        else:
            logger.log("No tasks found in plan.md", "WARNING")
        return True

    # チェーン開始ログ（タスク一覧含む）
    logger.log_chain_start(tasks)

    if manual_tasks:
        logger.log(f"Skipped {len(manual_tasks)} manual tasks:", "INFO")
        for task in manual_tasks:
            logger.log(f"  - [MANUAL] {task}", "INFO")
        logger.log_separator("-", 50)

    handover_content = None
    agent_number = 1

    while agent_number <= max_agents:
        # 毎回 plan.md を読み直して最新の状態を取得
        _, remaining_tasks, skipped_tasks = check_plan_completion(plan_path, skip_manual=skip_manual)

        if not remaining_tasks:
            if skipped_tasks:
                logger.log(f"All automated tasks completed. {len(skipped_tasks)} manual tasks remain.", "INFO")
            else:
                logger.log("All tasks already completed in plan.md", "INFO")
            logger.log_chain_end(success=True, total_agents=agent_number - 1)
            return True

        ctx = AgentContext(
            agent_number=agent_number,
            tasks=remaining_tasks,  # 残タスクのみ（手動タスク除外済み）
            work_dir=str(work_dir),
        )

        completed, new_handover = await run_agent(ctx, logger, plan_path, handover_content)

        if completed:
            logger.log_chain_end(success=True, total_agents=agent_number)
            return True

        if new_handover:
            # 引継ぎ書を保存
            handover_path.write_text(new_handover, encoding='utf-8')
            logger.log_handover(agent_number, agent_number + 1, "Context threshold exceeded")
            logger.log(f"Handover saved to: {handover_path}")

            handover_content = new_handover
            agent_number += 1
        else:
            # 引き継ぎなしで終了 = 致命的エラー（ロック等）
            logger.log("Agent stopped without handover (fatal error - file lock?)", "FATAL")
            logger.log_chain_end(success=False, total_agents=agent_number)
            return False

    logger.log(f"Max agents ({max_agents}) reached", "WARNING")
    logger.log_chain_end(success=False, total_agents=agent_number)
    return False


def parse_args():
    """コマンドライン引数を解析"""
    import argparse
    parser = argparse.ArgumentParser(
        description="Agent Runner - plan.md のタスクを自律的に実行",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
例:
  py scripts/agent_runner.py                    # 通常実行（手動タスクスキップ）
  py scripts/agent_runner.py --include-manual   # 手動タスクも含める
  py scripts/agent_runner.py --max-agents 5     # 最大5エージェントまで
  py scripts/agent_runner.py --status           # 現在の進捗を表示
  py scripts/agent_runner.py --clear-log        # ログをクリアして実行
"""
    )
    parser.add_argument(
        "--include-manual",
        action="store_true",
        help="手動タスク（フロントエンド、ブラウザ等）も含める"
    )
    parser.add_argument(
        "--max-agents",
        type=int,
        default=10,
        help="最大エージェント数（デフォルト: 10）"
    )
    parser.add_argument(
        "--status",
        action="store_true",
        help="現在の進捗を表示して終了"
    )
    parser.add_argument(
        "--clear-log",
        action="store_true",
        help="ログファイルをクリアしてから実行"
    )
    return parser.parse_args()


def show_status(plan_path: Path, skip_manual: bool = True):
    """現在の進捗を表示"""
    if not plan_path.exists():
        print(f"Error: {plan_path} not found")
        return

    completed, remaining, skipped = check_plan_completion(plan_path, skip_manual=skip_manual)
    total = len(completed) + len(remaining) + len(skipped)

    print(f"\nProgress: {len(completed)}/{total} tasks completed")
    print(f"   [x] Completed: {len(completed)}")
    print(f"   [ ] Remaining: {len(remaining)}")
    if skipped:
        print(f"   [-] Manual (skipped): {len(skipped)}")

    if remaining:
        print(f"\nRemaining tasks:")
        for i, task in enumerate(remaining, 1):
            print(f"   {i}. [ ] {task}")

    if skipped:
        print(f"\nManual tasks (require user action):")
        for task in skipped:
            print(f"   - {task}")

    print()


async def main():
    """メインエントリーポイント"""
    args = parse_args()

    # パス設定
    base_dir = Path(__file__).parent.parent
    plan_path = base_dir / PLAN_FILE
    work_dir = base_dir
    log_path = base_dir / LOG_FILE
    handover_path = base_dir / HANDOVER_FILE

    skip_manual = not args.include_manual

    # --status: 進捗表示のみ
    if args.status:
        show_status(plan_path, skip_manual=skip_manual)
        return 0

    # --clear-log: ログクリア
    if args.clear_log and log_path.exists():
        log_path.unlink()
        print(f"Cleared: {log_path}")

    print(f"Agent Runner")
    print(f"  Plan: {plan_path}")
    print(f"  Work Dir: {work_dir}")
    print(f"  Log: {log_path}")
    print(f"  Threshold: {THRESHOLD*100:.0f}%")
    print(f"  Max Agents: {args.max_agents}")
    print(f"  Skip Manual Tasks: {skip_manual}")
    print()

    success = await run_agent_chain(
        plan_path=plan_path,
        work_dir=work_dir,
        log_path=log_path,
        handover_path=handover_path,
        max_agents=args.max_agents,
        skip_manual=skip_manual,
    )

    if success:
        print("\n[OK] All tasks completed!")
        # 完了後に手動タスクがあれば表示
        _, _, skipped = check_plan_completion(plan_path, skip_manual=True)
        if skipped:
            print(f"\n[!] {len(skipped)} manual tasks require user action:")
            for task in skipped:
                print(f"   - {task}")
    else:
        print("\n[ERROR] Agent chain stopped.")
        print("  Possible causes:")
        print("  - File locked by another process (close running exe)")
        print("  - Permission denied (run as administrator)")
        print("  - Max agents reached")
        print(f"  Check {log_path} for details.")

    return 0 if success else 1


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
