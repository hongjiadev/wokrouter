# Product

## Register

product

## Platform

web

## Users

WokRouter 面向在本机使用 Codex CLI/App/SDK、Claude Code、GitHub Copilot App 与 OpenAI-compatible 客户端的开发者。他们通常在编码过程中快速确认代理是否运行、Provider 与账号是否健康、请求为何失败，并在不中断工作流的前提下完成恢复或切换。

## Product Purpose

WokRouter 以一个跨平台 Rust daemon 统一代理、路由、账号、客户端集成和本地运行状态，CLI 与 Tauri 桌面端共享同一控制面。成功意味着用户能从一个可信界面判断系统当前状态、理解下一步动作，并安全完成修复；关闭桌面端不会影响 daemon，默认不上传遥测或持久化请求正文。

## Positioning

一个隐私优先、契约驱动的本地 AI 路由控制面：以单一 daemon 提供可核验的运行状态和一致操作，而不是另一个通用 AI IDE 或账号多开工具。

## Brand Personality

克制、精确、可信。界面语气像可靠的本地运维工具：直接说明状态、后果和恢复动作，不用营销式夸张，也不把技术细节变成视觉噪声。

## Anti-references

- 不做 CC Switch 或 Cockpit Tools 式的通用 AI IDE 账号/多开管理器；只参考其公开产品体验与信息架构，不复制 Cockpit Tools 的代码、文案或资产。
- 不使用泛 SaaS 仪表盘的重复卡片网格、装饰性渐变、玻璃拟态、霓虹 AI 紫或与任务无关的动效。
- 不把配置表单放在运行状态之前，也不以成功色掩盖未知、恢复中或失联状态。

## Design Principles

1. **运行状态优先。** 首屏先回答 daemon 是否可用、当前版本和下一步动作，再逐层进入配置。
2. **状态必须诚实。** running、stopped、loading、error 和 recovery 各有明确语义；未知状态不能伪装成健康。
3. **操作围绕恢复。** 错误信息给出安全、可执行的恢复路径，破坏性或扩大网络范围的动作必须显式确认。
4. **隐私可见但不喧宾夺主。** 密钥和正文从不出现在普通界面、日志或诊断；元数据的范围清楚可核验。
5. **国际化默认成立。** 自动检测 locale 与 IANA timezone，不设置首次选择门槛；布局从一开始支持 RTL、长文本与本地化数字。

## Accessibility & Inclusion

所有核心流程必须支持键盘操作、清晰焦点、语义化状态与屏幕放大；在 200% zoom 和窄窗口下仍可完成任务。动效遵循 reduced-motion，颜色不作为唯一状态编码，正文与控件保持可读对比度。RTL locale 正确镜像结构但不反转数字、模型 ID、端口或协议文本。
