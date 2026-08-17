import { useEffect, useState } from 'react';
import { PageContainer } from '@ant-design/pro-components';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Alert, Button, Card, Empty, Input, List, Space, Typography, message } from 'antd';
import { SaveOutlined } from '@ant-design/icons';
import { adminApi, ApiError } from '../lib/api';

export function WorkspacePage() {
  const queryClient = useQueryClient();
  const [pathInput, setPathInput] = useState('AGENTS.md');
  const [selectedPath, setSelectedPath] = useState('');
  const [content, setContent] = useState('');
  const [dirty, setDirty] = useState(false);

  const filesQuery = useQuery({
    queryKey: ['workspaceFiles'],
    queryFn: adminApi.workspaceFiles,
  });
  const fileQuery = useQuery({
    queryKey: ['workspaceFile', selectedPath],
    queryFn: () => adminApi.workspaceFile(selectedPath),
    enabled: Boolean(selectedPath),
    retry: false,
  });

  useEffect(() => {
    if (fileQuery.data) {
      setContent(fileQuery.data.content);
      setDirty(false);
    } else if (
      selectedPath &&
      fileQuery.error instanceof ApiError &&
      fileQuery.error.status === 404
    ) {
      setContent('');
      setDirty(false);
    }
  }, [fileQuery.data, fileQuery.error, selectedPath]);

  const saveMutation = useMutation({
    mutationFn: () => adminApi.saveWorkspaceFile(selectedPath, content),
    onSuccess: async () => {
      setDirty(false);
      message.success('工作区文件已保存');
      await queryClient.invalidateQueries({ queryKey: ['workspaceFiles'] });
      await queryClient.invalidateQueries({ queryKey: ['workspaceFile', selectedPath] });
    },
    onError: (error) => {
      message.error(error instanceof Error ? error.message : '保存失败');
    },
  });

  const openPath = (path: string) => {
    const next = path.trim();
    if (!next) return;
    setPathInput(next);
    setSelectedPath(next);
  };

  return (
    <PageContainer
      title="提示词与工作区"
      subTitle="管理新会话可见的文本文件"
      extra={
        <Button
          type="primary"
          icon={<SaveOutlined />}
          disabled={!selectedPath || !dirty}
          loading={saveMutation.isPending}
          onClick={() => saveMutation.mutate()}
        >
          保存
        </Button>
      }
    >
      <Alert
        type="info"
        showIcon
        message="文件限制在 OPENAB_WORKSPACE_ROOT 内；支持 AGENTS.md、系统提示词及其他文本配置，单文件最大 1 MiB。"
      />
      <div className="workspace-manager">
        <Card className="workspace-file-panel" title="文件">
          <Space.Compact block>
            <Input
              value={pathInput}
              placeholder="例如 AGENTS.md"
              onChange={(event) => setPathInput(event.target.value)}
              onPressEnter={() => openPath(pathInput)}
            />
            <Button onClick={() => openPath(pathInput)}>打开或新建</Button>
          </Space.Compact>
          <List
            className="workspace-file-list"
            loading={filesQuery.isLoading}
            dataSource={filesQuery.data || []}
            locale={{ emptyText: <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="暂无文件" /> }}
            renderItem={(file) => (
              <List.Item>
                <Button
                  type={selectedPath === file.path ? 'primary' : 'text'}
                  block
                  className="workspace-file-button"
                  onClick={() => openPath(file.path)}
                >
                  <span>{file.path}</span>
                  <Typography.Text type="secondary">{file.size} B</Typography.Text>
                </Button>
              </List.Item>
            )}
          />
        </Card>
        <Card
          className="workspace-editor-panel"
          title={selectedPath || '选择或新建文件'}
        >
          {selectedPath ? (
            <Input.TextArea
              className="workspace-editor"
              value={content}
              onChange={(event) => {
                setContent(event.target.value);
                setDirty(true);
              }}
              placeholder="输入提示词或工作区说明"
              spellCheck={false}
            />
          ) : (
            <Empty description="从左侧选择文件，或输入安全的相对路径新建文件" />
          )}
        </Card>
      </div>
    </PageContainer>
  );
}

