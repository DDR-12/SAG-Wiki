import catalog from './llm-providers.json';

export type ModelKind = 'answer' | 'jev';
export type ProviderApi = 'openai' | 'anthropic' | 'gemini' | 'ollama';
type Provider = {value: string; label: string; url: string};
export type LlmProvider = Provider & {api: ProviderApi; hosts: string[]; matchPort?: number; presets: string[]};

export const llmProviders: LlmProvider[] = catalog.providers.map(provider => ({
 value: provider.id,
 label: provider.label,
 url: provider.base_url,
 api: provider.api as ProviderApi,
 hosts: provider.hosts,
 matchPort: provider.match_port,
 presets: provider.presets,
}));

export const providers: Record<ModelKind, Provider[]> = {
 answer: llmProviders.map(({value, label, url}) => ({value, label, url})),
 jev: [
  {value:'zleap',label:'Zleap',url:'https://api.zleap.com/v1'},
  {value:'302',label:'302.AI',url:'https://api.302ai.cn/typesafeai/v1'},
  {value:'openrouter',label:'OpenRouter',url:'https://openrouter.ai/api/alpha/decisions'},
  {value:'official',label:'Typesafe',url:'https://api.typesafe.ai/v1'},
  {value:'custom',label:'自定义',url:''},
 ],
};

export function presetsFor(id: string): string[] {
 return llmProviders.find(provider => provider.value === id)?.presets ?? [];
}

export function providerFor(kind: ModelKind, address: string): string {
 let url: URL;
 try { url = new URL(address); } catch { return 'custom'; }
 if (kind === 'jev') {
  if (['api.302.ai','api.302ai.cn'].includes(url.hostname)) return '302';
  return providers.jev.find(provider => provider.url && new URL(provider.url).hostname === url.hostname)?.value ?? 'custom';
 }
 return llmProviders.find(provider => {
  if (!provider.hosts.includes(url.hostname)) return false;
  return provider.matchPort == null || url.port === String(provider.matchPort);
 })?.value ?? 'custom';
}

export function sameProviderOrigin(a: string, b: string): boolean {
 try { return new URL(a).origin === new URL(b).origin; } catch { return false; }
}

/** OpenRouter 和自定义端点接受任意模型 ID，切换时不改已填的名称。 */
export const freeFormAnswerProviders = new Set(['custom', 'openrouter']);

export function alignAnswerModel(provider: string, current: string, manual: string[], known: string[]): {model: string; models: string[]} {
 const selected = current.trim();
 const lines = unique(manual);
 if (freeFormAnswerProviders.has(provider)) return {model: selected, models: lines.length ? lines : (selected ? [selected] : [])};
 const allowed = unique(known);
 const next = selected && allowed.includes(selected) ? selected : (allowed[0] ?? '');
 const kept = lines.filter(item => allowed.includes(item));
 if (next && !kept.includes(next)) kept.unshift(next);
 return {model: next, models: unique(kept)};
}

export function modelChoices(provider: string, remote: string[] | null, manual: string[]): {source: 'remote' | 'preset'; models: string[]} {
 const extra = manual.map(item => item.trim()).filter(Boolean);
 if (remote && remote.length) return {source: 'remote', models: unique([...remote, ...extra])};
 return {source: 'preset', models: unique([...presetsFor(provider), ...extra])};
}

export function redactSecret(message: string, secret: string): string {
 const value = secret.trim();
 if (!value) return message.replace(/^Error:\s*/, '');
 return message.replace(/^Error:\s*/, '').split(value).join('[已隐藏]');
}

export function maskKey(key: string): string {
 const value = key.trim();
 if (!value) return '';
 return '•'.repeat(Math.min(Math.max(value.length, 8), 16));
}

const SECRET_FIELDS = ['api_key', 'apiKey', 'api_token', 'access_token', 'secret', 'key'];
const URL_FIELDS = ['base_url', 'baseUrl', 'api_base', 'apiBase', 'endpoint'];
const MODEL_FIELDS = ['model_name', 'modelName', 'model_id', 'modelId'];

export function migrateModelConfig(raw: Record<string, unknown> | undefined, kind: ModelKind): {config: Record<string, unknown>; key?: string} {
 const source = {...(raw ?? {})};
 let key = '';
 for (const field of SECRET_FIELDS) {
  const value = source[field];
  if (typeof value === 'string' && value.trim() && !key) key = value.trim();
  delete source[field];
 }
 if (typeof source.url !== 'string' || !source.url.trim()) {
  for (const field of URL_FIELDS) {
   if (typeof source[field] === 'string' && source[field].trim()) { source.url = source[field]; break; }
  }
 }
 for (const field of URL_FIELDS) delete source[field];
 if (typeof source.model !== 'string' || !source.model.trim()) {
  for (const field of MODEL_FIELDS) {
   if (typeof source[field] === 'string' && source[field].trim()) { source.model = source[field]; break; }
  }
 }
 for (const field of MODEL_FIELDS) delete source[field];
 if (typeof source.models === 'string') source.models = unique(source.models.split(/[\n,]/));
 else if (!Array.isArray(source.models) && (typeof source.available_models === 'string' || Array.isArray(source.available_models))) source.models = source.available_models;
 if (typeof source.models === 'string') source.models = unique(source.models.split(/[\n,]/));
 if (Array.isArray(source.models)) source.models = unique(source.models.map(item => String(item)));
 delete source.available_models;
 delete source.availableModels;
 if (typeof source.provider !== 'string' || !source.provider.trim()) {
  source.provider = typeof source.vendor === 'string' && source.vendor.trim() ? source.vendor : providerFor(kind, String(source.url ?? ''));
 }
 delete source.vendor;
 if (typeof source.url === 'string') {
  const normalized = normalizeAnswerUrl(String(source.provider ?? ''), source.url);
  if (normalized !== source.url) source.url = normalized;
 }
 if (key) source.has_key = true;
 return {config: source, key: key || undefined};
}

export function normalizeAnswerUrl(provider: string, url: string): string {
 const trimmed = url.trim();
 let parsed: URL;
 try { parsed = new URL(trimmed); } catch { return trimmed; }
 if (parsed.hostname !== 'api.anthropic.com') return trimmed;
 const path = parsed.pathname.replace(/\/+$/, '');
 if (path !== '' && path !== '/') return trimmed;
 if (provider !== 'anthropic' && provider !== '') return trimmed;
 parsed.pathname = '/v1';
 parsed.search = '';
 parsed.hash = '';
 return parsed.toString();
}

function unique(values: string[]): string[] {
 const seen = new Set<string>();
 const result: string[] = [];
 for (const value of values.map(item => item.trim()).filter(Boolean)) {
  if (seen.has(value)) continue;
  seen.add(value);
  result.push(value);
 }
 return result;
}
