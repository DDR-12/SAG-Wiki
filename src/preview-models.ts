import {mount} from 'svelte';
import SettingsPanel from './components/Settings.svelte';
import type {Settings} from './lib/types';

const saved=new URLSearchParams(location.search).get('saved')==='1';
const value: Settings = {
 answer: {provider: 'openai', url: 'https://api.openai.com/v1', model: 'gpt-4o-mini', models: ['gpt-4o-mini'], has_key: saved, auth_mode: 'bearer'},
 jev: {provider: '302', url: 'https://api.302ai.cn/typesafeai/v1', model: 'jev-latest', models: [], has_key: false, auth_mode: 'bearer'},
 retrieval: {candidate_k: 50, seed_k: 10, expansion_term_limit: 20, max_hops: 1, top_k: 10, chunk_chars: 512},
 theme: 'light',
 start_page: 'ask',
 editor_size: 15,
 show_hidden: false,
 watch: true,
 excluded: [],
 timeout_secs: 120,
 answer_context_tokens: 65536,
 answer_output_tokens: 2048,
};

mount(SettingsPanel, {
 target: document.getElementById('app')!,
 props: {value, initial: 'models', onClose: () => {}, onSave: async () => {}},
});
