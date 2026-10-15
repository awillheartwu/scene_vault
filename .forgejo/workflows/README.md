# Forgejo CI disabled

Scene Vault 的自动构建与发布仅在 GitHub Actions 执行，Forgejo 用于同步代码。

保留这个目录及说明文件，不在此添加 `.yml` 或 `.yaml` 工作流。Forgejo 15 发现此目录
后会停止查找其他工作流目录，即使这里没有工作流，也不会回退读取 `.github/workflows`。
Git 不保存空目录，因此需要保留本文件。

依据：[Forgejo 工作流目录规则](https://forgejo.org/docs/v15.0/user/actions/overview/#workflows)。
