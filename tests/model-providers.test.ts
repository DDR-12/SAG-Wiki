import {readFileSync} from 'node:fs';
import {expect, it} from 'vitest';
import {alignAnswerModel, llmProviders, maskKey, migrateModelConfig, modelChoices, normalizeAnswerUrl, presetsFor, providerFor, redactSecret} from '../src/lib/providers';

const required = [
 ['openai', 'https://api.openai.com/v1'],
 ['anthropic', 'https://api.anthropic.com/v1'],
 ['gemini', 'https://generativelanguage.googleapis.com/v1beta/openai'],
 ['deepseek', 'https://api.deepseek.com'],
 ['dashscope', 'https://dashscope.aliyuncs.com/compatible-mode/v1'],
 ['glm', 'https://open.bigmodel.cn/api/paas/v4'],
 ['moonshot', 'https://api.moonshot.cn/v1'],
 ['siliconflow', 'https://api.siliconflow.cn/v1'],
 ['openrouter', 'https://openrouter.ai/api/v1'],
 ['ollama', 'http://127.0.0.1:11434/v1'],
 ['custom', ''],
] as const;

it('供应商注册表包含默认地址和预设模型', () => {
 for (const [id, url] of required) {
  const provider = llmProviders.find(item => item.value === id);
  expect(provider?.url).toBe(url);
  if (id !== 'custom') expect(presetsFor(id).length).toBeGreaterThan(0);
 }
 expect(llmProviders.some(provider => provider.value === '302')).toBe(true);
 expect(providerFor('answer', 'https://api.moonshot.cn/v1')).toBe('moonshot');
 expect(providerFor('answer', 'http://127.0.0.1:11434/v1')).toBe('ollama');
 expect(providerFor('answer', 'http://127.0.0.1:8000/v1')).toBe('custom');
 expect(providerFor('jev', 'https://api.zleap.com/v1')).toBe('zleap');
 expect(providerFor('jev', 'https://api.typesafe.ai/v1')).toBe('official');
});

it('模型列表成功时使用远端结果，失败时退回预设并保留手工输入', () => {
 expect(modelChoices('openai', ['remote-a', 'remote-b'], ['hand-written', 'remote-a'])).toEqual({source: 'remote', models: ['remote-a', 'remote-b', 'hand-written']});
 expect(modelChoices('openai', null, ['hand-written'])).toEqual({source: 'preset', models: [...presetsFor('openai'), 'hand-written']});
 expect(modelChoices('openai', [], ['hand-written']).source).toBe('preset');
});

it('旧模型配置迁移保留字段并抽出密钥', () => {
 const migrated = migrateModelConfig({
  base_url: 'https://api.deepseek.com',
  model_name: 'deepseek-chat',
  models: 'deepseek-chat, deepseek-reasoner',
  api_key: 'legacy-key',
  reasoning_effort: 'high',
  note: 'keep-me',
 }, 'answer');
 expect(migrated.key).toBe('legacy-key');
 expect(migrated.config).toMatchObject({
  url: 'https://api.deepseek.com',
  model: 'deepseek-chat',
  models: ['deepseek-chat', 'deepseek-reasoner'],
  provider: 'deepseek',
  has_key: true,
  reasoning_effort: 'high',
  note: 'keep-me',
 });
 expect(migrated.config).not.toHaveProperty('api_key');
 expect(JSON.stringify(migrated.config)).not.toContain('legacy-key');
});

it('封闭供应商会换掉不在列表中的模型，OpenRouter 和自定义保留原名', () => {
 expect(alignAnswerModel('anthropic', 'gpt-4o-mini', ['gpt-4o-mini'], presetsFor('anthropic'))).toEqual({model: 'claude-sonnet-4-5', models: ['claude-sonnet-4-5']});
 expect(alignAnswerModel('openai', 'gpt-4o', ['gpt-4o', 'gpt-4o-mini'], presetsFor('openai')).model).toBe('gpt-4o');
 expect(alignAnswerModel('custom', 'gpt-4o-mini', ['gpt-4o-mini'], [])).toEqual({model: 'gpt-4o-mini', models: ['gpt-4o-mini']});
 expect(alignAnswerModel('openrouter', 'gpt-4o-mini', ['gpt-4o-mini'], presetsFor('openrouter'))).toEqual({model: 'openai/gpt-4o-mini', models: ['openai/gpt-4o-mini']});
 expect(alignAnswerModel('openrouter', 'anthropic/claude-sonnet-4.5', ['anthropic/claude-sonnet-4.5'], presetsFor('openrouter')).model).toBe('anthropic/claude-sonnet-4.5');
 expect(alignAnswerModel('ollama', '', [], [])).toEqual({model: '', models: []});
});

it('不带版本的 Anthropic 地址会补上 /v1', () => {
 expect(normalizeAnswerUrl('anthropic', 'https://api.anthropic.com')).toBe('https://api.anthropic.com/v1');
 expect(normalizeAnswerUrl('anthropic', 'https://api.anthropic.com/')).toBe('https://api.anthropic.com/v1');
 expect(normalizeAnswerUrl('anthropic', 'https://api.anthropic.com/v1')).toBe('https://api.anthropic.com/v1');
 expect(normalizeAnswerUrl('anthropic', 'https://proxy.example/v1')).toBe('https://proxy.example/v1');
 expect(normalizeAnswerUrl('custom', 'https://gateway.example.com')).toBe('https://gateway.example.com');
 expect(migrateModelConfig({provider: 'anthropic', url: 'https://anthropic-proxy.example/v1', model: 'claude-sonnet-4-5'}, 'answer').config.url).toBe('https://anthropic-proxy.example/v1');
 const migrated = migrateModelConfig({provider: 'anthropic', url: 'https://api.anthropic.com', model: 'claude-sonnet-4-5'}, 'answer');
 expect(migrated.config.url).toBe('https://api.anthropic.com/v1');
 expect(migrated.config.model).toBe('claude-sonnet-4-5');
});

it('密钥掩码和错误脱敏不回显原文', () => {
 expect(maskKey('sk-test-secret')).toBe('•'.repeat('sk-test-secret'.length));
 expect(maskKey('sk-test-secret')).not.toContain('sk-');
 expect(maskKey('')).toBe('');
 expect(redactSecret('Error: 模型请求失败：sk-test-secret', 'sk-test-secret')).toBe('模型请求失败：[已隐藏]');
 expect(redactSecret('模型请求失败：sk-test-secret', 'sk-test-secret')).not.toContain('sk-test-secret');
});

it('示例配置和 gitignore 不保存真实密钥', () => {
 const ignore = readFileSync(new URL('../.gitignore', import.meta.url), 'utf8');
 expect(ignore).toMatch(/^credentials\.json$/m);
 expect(ignore).toMatch(/^\.env\*$/m);
 const example = readFileSync(new URL('../config/model.example.json', import.meta.url), 'utf8');
 expect(example).not.toMatch(/sk-|api_key|secret|token/i);
 const parsed = JSON.parse(example);
 expect(parsed.answer.url).toBe('https://api.openai.com/v1');
 expect(parsed.answer.model).toBe('gpt-4o-mini');
 expect(parsed).not.toHaveProperty('key');
});
