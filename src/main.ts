import { mount } from 'svelte';
import App from './App.svelte';
import QuickSearch from './QuickSearch.svelte';
import './tokens.css';
import './prototype.css';
import './app.css';
import './native.css';
import './styles/search.css';
import './components/settings.css';
const preview=import.meta.env.DEV&&new URLSearchParams(location.search).get('preview')==='models';
if(preview)await import('./preview-models');
else mount(new URLSearchParams(location.search).has('quick-search')?QuickSearch:App,{target:document.getElementById('app')!});

import './components/ui/controls.css';
