# AUTO-MAS CLI 契约 v1.1

## 1. 文档状态

- 本文档定义首个独立 CLI 的外部契约。
- 本契约与实现语言无关，当前 Python CLI 为参考实现。
- 后续若用 Rust 重写 CLI，应先继承本文档，再讨论扩展或修订。

## 2. 目标

- 保持 CLI 为后端能力之上的薄封装，而不是重新实现后端业务。
- 稳定命令名、参数名、输出格式与退出码，便于脚本调用与后续重写。
- 将 CLI 分发问题与后端服务问题解耦。

## 2.1 术语说明

- 本文中的 `backend start` 指“启动后端进程本身”。
- 本文中的“启动队列”或“启动任务”指“在后端已运行的前提下，通过 API 创建执行实例并开始调度”。
- 两者不是同一层能力，不能混用。

## 3. 非目标

- 不定义后端内部模块拆分方式。
- v1.1 不要求 WebSocket 能力。
- v1.1 不覆盖安装器实现细节，只约束安装后应具备的行为。
- v1.1 不承诺任务级、脚本级运行控制契约。

## 4. v1.1 范围

顶层二进制名：

```text
auto-mas-cli
```

v1.1 必选命令：

```text
auto-mas-cli backend status
auto-mas-cli backend start
auto-mas-cli backend stop
auto-mas-cli queue list
auto-mas-cli queue start --queue-id <id> [--mode <mode>]
```

暂缓命令：

```text
script list
power get
power set
power cancel
```

## 5. 全局参数

支持的全局参数：

```text
--api-url <url>
--app-root <path>
--python-exe <path>
--json
--no-auto-start
--keep-backend
```

约束如下：

- 全局参数可以出现在命令组之前，也可以出现在命令组之后。
- `--api-url` 默认值为 `http://127.0.0.1:36163`。
- `--app-root` 仅对本地 `backend start` 生效，用于覆盖应用根目录自动发现。
- `--python-exe` 仅对本地 `backend start` 生效，用于覆盖 Python 解释器自动发现。
- `--json` 切换为机器可读输出。
- `--no-auto-start` 禁止命令隐式拉起后端。
- `--keep-backend` 仅对会自动拉起本地后端的命令生效，用于禁止 CLI 在命令结束后关闭自己拉起的后端。

## 6. 运行时解析契约

CLI 的运行时解析来源只有三类：

1. 显式命令行参数
2. 环境变量
3. 本地自动发现

环境变量：

```text
AUTO_MAS_ROOT
AUTO_MAS_PYTHON
```

应用根目录发现规则：

- 应用根目录必须至少包含 `main.py`、`app/`、`requirements.txt`。
- 解析优先级如下：
  1. `--app-root`
  2. `AUTO_MAS_ROOT`
  3. 当前工作目录
  4. CLI 包相对路径
  5. 可执行文件相邻位置

Python 解释器发现规则：

- 解析优先级如下：
  1. `--python-exe`
  2. `AUTO_MAS_PYTHON`
  3. `environment/python` 下的内置运行时
  4. 当前解释器（非冻结运行时）

## 7. 命令行为契约

### 7.1 CLI 编排类命令

这类命令由 CLI 自身负责调度、探活、进程拉起或关闭，不等价于某个公开后端 `*Out` 模型。

`backend status`：

- 这是 CLI 探活命令，不是后端业务接口本身。
- 必须永远禁止自动拉起后端。
- 可用于本地后端，也可用于远端后端。
- 即使后端未运行，也应正常返回“后端不可用”的状态结果，而不是把“未运行”视为命令执行失败。
- 当后端未运行时，命令仍应以退出码 `0` 返回，并给出清晰、温和的用户提示。
- 当前实现通过 `POST /api/info/version` 探测后端是否可达，但该接口返回的 `VersionOut` 仅作为探针，不直接等价于命令输出。

`backend start`：

- 这是 CLI 编排命令，不存在对应的公开后端启动接口。
- 该命令只支持本地后端启动，不提供远端启动能力。
- 若后端已可用，命令应幂等成功。
- 命令成功的判定标准不是“已拉起本地进程”，而是“探活成功”，即 CLI 在启动后应通过 `POST /api/info/version` 确认后端已就绪。
- 命令结束后，后端必须继续保持运行。

`backend stop`：

- 这是 CLI 编排命令，不应因为用户执行停止而反向触发自动启动。
- 该命令可用于本地后端，也可直接用于当前 `--api-url` 指向的远端后端。
- 若后端可用，CLI 应向 `POST /api/core/close` 发送关闭请求。
- 对本地且由 CLI 跟踪的后端进程，CLI 可以继续等待退出作为附加清理步骤；对远端后端，不要求等待探活失败或确认进程退出。
- 命令成功的判定标准是“关闭请求已成功发出并被接受”，而不是“目标后端已经完全退出”。
- 若后端未运行，应返回明确的用户可读错误。

### 7.2 API 直连类命令

这类命令直接映射到现有后端 HTTP 接口，成功时 JSON 输出应尽量保留后端原始响应字段。

`queue list`：

- 默认允许自动拉起后端，除非显式传入 `--no-auto-start`。
- 若是 CLI 为本次命令临时拉起的后端，命令完成后可以自动关闭，除非传入 `--keep-backend`。

`queue start`：

- 这是 API 直连命令，用于在后端已运行（或可自动拉起）的前提下启动指定队列执行。
- 默认允许自动拉起后端，除非显式传入 `--no-auto-start`。
- 命令参数：
  - `--queue-id <id>`：必填，目标队列 ID。
  - `--mode <mode>`：可选，默认值为 `AutoProxy`。
- CLI 应调用 `POST /api/dispatch/start`，请求体格式如下：
  ```json
  {
    "mode": "AutoProxy",
    "taskId": "<queueId>"
  }
  ```
- 其中 `taskId` 字段在该命令语义下承载 queue ID；CLI 不应在本层重命名后端字段。
- 成功时 JSON 输出应优先透传后端响应字段（通常为 `OutBase` 风格）。

当前稳定契约保留 queue 级别可见控制入口（`queue list`、`queue start`），不对 task 级或脚本级运行控制做公开承诺。

### 7.3 信号与中断语义

- CLI 进程若被 `Ctrl+C`、终端关闭或其他外部信号中断，不应主动关闭后端。
- “用户中断 CLI” 与 “用户要求停止后端” 是两种不同语义，不能混用。
- 即使某次命令由 CLI 临时拉起了后端，异常中断时也应优先保留后端存活，由用户后续显式决定是否关闭。

## 8. 后端 API 映射

v1.1 依赖的后端接口如下：

```text
POST /api/info/version
POST /api/core/close
POST /api/queue/get
POST /api/dispatch/start
```

命令与接口的映射关系：

- `backend status`
  - 探针接口：`POST /api/info/version`
  - 后端返回模型：`VersionOut`
  - 注意：该模型只用于探活，不直接作为命令输出契约
- `backend start`
  - 无公开后端接口
  - CLI 负责本地进程拉起与就绪等待
- `backend stop`
  - 内部调用：`POST /api/core/close`
  - 后端返回模型：`OutBase`
  - 注意：该模型用于内部判定请求是否成功，CLI 对外仍可输出自己的停止确认结果
- `queue list`
  - 接口：`POST /api/queue/get`
  - 请求体：
    ```json
    {"queueId": null}
    ```
  - 后端返回模型：`QueueGetOut`
- `queue start`
  - 接口：`POST /api/dispatch/start`
  - 请求体：
    ```json
    {
      "mode": "AutoProxy",
      "taskId": "<queueId>"
    }
    ```
  - 后端返回模型：`OutBase`
  - 注意：`taskId` 为后端既有字段，CLI 在 queue 语义下将其映射为 queue ID 输入。

当前边界说明：

- `dispatch` 相关能力在 v1.1 中仅公开 queue 级入口（`queue start`）。
- 现阶段 CLI 稳定承诺 backend 级与 queue 级能力。
- `backend stop` 可作为远端控制能力使用，但 `backend start` 仍仅限本地编排。
- 文档中“不支持后端启动接口”仅指“不支持通过公开 API 启动后端进程本身”；不影响通过 `dispatch/start` 启动队列执行。

## 9. 输出契约

CLI 必须同时支持两种输出模式：

1. 适合人工阅读的文本模式
2. 适合脚本消费的 JSON 模式

文本模式约束：

- 输出应尽量简洁、按行组织。
- 成功路径不输出堆栈。
- 错误信息写入 stderr。

JSON 模式约束：

- 输出必须是单个 JSON 对象。
- 对于 API 直连类命令，成功结果应尽量保留后端原始字段，不要重新发明同义字段。
- 对于 CLI 编排类命令，也应统一采用 `code`、`status`、`message`、`data` 包络。
- `--json` 模式下，无论成功还是失败，JSON 均写入 stdout。
- `--json` 模式下，不应再向 stdout 或 stderr 混入额外的人类文本。
- JSON 中的 `code` 始终表示契约结果码，不表示进程退出码；进程退出码只由 CLI 进程本身返回。

当前 v1.1 建议的 JSON 成功语义如下：

- `backend status`
  - 后端运行时：
    ```json
    {
      "code": 200,
      "status": "success",
      "message": "后端运行中",
      "data": {
        "ready": true,
        "startedByCli": false,
        "trackedPid": null,
        "appRoot": "/path/to/AUTO-MAS-Lite",
        "pythonExecutable": "/path/to/python",
        "apiUrl": "http://127.0.0.1:36163"
      }
    }
    ```
- `backend status`
  - 远端后端运行时：
    ```json
    {
      "code": 200,
      "status": "success",
      "message": "后端运行中",
      "data": {
        "ready": true,
        "startedByCli": null,
        "trackedPid": null,
        "appRoot": null,
        "pythonExecutable": null,
        "apiUrl": "https://example.com"
      }
    }
    ```
- `backend status`
  - 后端未运行时：
    ```json
    {
      "code": 200,
      "status": "success",
      "message": "后端未运行",
      "data": {
        "ready": false,
        "startedByCli": false,
        "trackedPid": null,
        "appRoot": "/path/to/AUTO-MAS-Lite",
        "pythonExecutable": "/path/to/python",
        "apiUrl": "http://127.0.0.1:36163"
      }
    }
    ```
- `backend start`
  - 返回完成探活后的运行时信息，例如：
    ```json
    {
      "code": 200,
      "status": "success",
      "message": "后端已就绪",
      "data": {
        "ready": true,
        "startedByCli": true,
        "trackedPid": 12345,
        "appRoot": "/path/to/AUTO-MAS-Lite",
        "pythonExecutable": "/path/to/python",
        "apiUrl": "http://127.0.0.1:36163"
      }
    }
    ```
- `backend stop`
  - 返回统一包络结果，例如：
    ```json
    {
      "code": 200,
      "status": "success",
      "message": "后端关闭请求已发送",
      "data": {
        "requestAccepted": true,
        "target": "local"
      }
    }
    ```
- `queue list`
  - 直接透传 `QueueGetOut`
- `queue start`
  - 直接透传 `OutBase`

文本模式示例：

```text
backend: running
apiUrl: http://127.0.0.1:36163
appRoot: /path/to/AUTO-MAS-Lite
python: /path/to/python
```

```text
queueId	name
<queue-id>	默认队列
```

```text
queue start accepted
queueId: <queue-id>
mode: AutoProxy
```

```text
backend: stopped
message: 后端未运行
apiUrl: http://127.0.0.1:36163
```

```text
后端关闭请求已发送
target: remote
apiUrl: https://example.com
```

## 10. 错误契约

面向用户的错误可归一到以下类别：

- `backend_unreachable`
- `backend_startup_failed`
- `invalid_arguments`
- `backend_business_error`
- `invalid_runtime_configuration`

对齐原则：

- 如果错误来自后端，并且后端已经返回标准 `OutBase` 风格字段，则 JSON 输出应优先保留 `code`、`status`、`message`。
- 如果错误发生在 CLI 本地，例如运行时解析失败、HTTP 连接失败、后端启动超时，则 CLI 可在 JSON 中补充 `source`、`category` 等附加字段，但不应覆盖后端原始错误字段。
- CLI 本地错误中的 `code` 仍表示契约结果码，不表示进程退出码；退出码继续由进程级返回值表达。

CLI 本地错误的推荐 JSON 结构：

```json
{
  "code": 500,
  "status": "error",
  "message": "后端未运行，无法执行命令",
  "source": "cli",
  "category": "backend_unreachable"
}
```

文本模式错误结构：

```text
错误: <message>
```

当前 Python 参考实现说明：

- `argparse` 解析失败时，当前实现会直接输出帮助信息并以退出码 `2` 结束。
- 当前 Python 参考实现尚未完全对齐本文档：CLI 编排类命令的 JSON 成功包络、CLI 本地失败的 JSON 输出、以及信号中断后的后端保活策略仍需补齐。
- 若参考实现中仍存在实验性的 task 相关命令，它们不属于本文档定义的稳定公开契约。
- 当前 Python 参考实现仍主要偏向本地后端编排，对远端 `backend stop` 的公开契约对齐也尚未补齐。

## 11. 退出码

稳定退出码：

- `0`：成功
- `1`：通用 CLI、运行时或后端错误
- `2`：参数解析错误

预留退出码：

- `10`：后端不可达
- `11`：后端启动失败
- `12`：后端业务错误

v1.1 Python 参考实现当前仍可将大部分运行时与后端失败折叠为 `1`，但后续实现不应改变命令语义。

## 12. 分发契约

L0 开发模式：

- 调用方式：`python -m app.cli`

L1 便携模式：

- 允许提供打包后的 CLI 可执行文件或启动器。
- 用户可以通过 `AUTO_MAS_ROOT` 与 `AUTO_MAS_PYTHON` 指向目标运行时。
- 不要求安装器。

L2 安装器模式：

- 安装器必须将 CLI 安装目录加入用户 `PATH`。
- 卸载时必须移除自己新增的 `PATH` 项。
- Windows `App Paths` 支持可选，不是 v1.1 必选项。

L3 产品化模式：

- 需要补齐 CLI 版本管理、升级流程与签名。
- 后端生命周期可以逐步演进为服务模式，但上文定义的命令语义不能被悄悄改写。

## 13. 兼容性规则

- 一旦发布，命令名与参数名即视为稳定。
- API 直连类命令在 JSON 模式下，应优先复用现有后端响应字段。
- 可以新增命令，但不应改变已发布命令的语义。
- 若后端 API 契约发生变化，应先更新契约文档，再修改 CLI 实现。
- 后端兼容性协商机制可后续单独引入，但不属于 v1.1 CLI 客户端必须承担的职责。

## 14. Rust 重写边界

如果后续引入 Rust CLI，应保持以下边界：

- Rust CLI 负责参数解析、输出格式化、运行时发现、安装器集成与本地进程编排。
- Python 后端继续负责任务执行与业务逻辑。
- Rust CLI 只能通过稳定的后端 API 或显式版本化的控制协议与后端通信。
- 不应在 CLI 中复制或迁移后端业务逻辑，除非后端架构本身同步重构。

## 15. 与 `mas-api-contract` 对齐说明

- 后端 HTTP 接口仍以 `*In` / `*Out` 模型为准，例如 `QueueGetOut`、`OutBase`。
- 当前 CLI 稳定契约消费 backend / queue 相关接口；`queue start` 允许通过 `dispatch/start` 触发队列运行，但不扩展为通用 task 级公开契约。
- 后端标准错误字段仍以 `code`、`status`、`message` 为核心。
- CLI 只有在执行“编排类命令”时，才允许输出不对应某个后端 `*Out` 的本地结果对象。
- 即使 CLI 需要添加附加字段，也应采用增量方式，不应发明与后端已有语义冲突的新错误包络。
