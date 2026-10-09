#!/usr/bin/python3
import fcntl,json,os,sys
root=os.environ['HOME'];lock=open(root+'/herdr-fixture.lock','w');fcntl.flock(lock,fcntl.LOCK_EX);path=root+'/herdr-state.json';a=sys.argv[1:]
s=json.load(open(path)) if os.path.exists(path) else {'creates':0,'starts':0,'prompts':[],'live':False,'agent':False,'accepted':False}
if os.path.exists(root+'/coordinator-status'):s['status']=open(root+'/coordinator-status').read().strip()
def pane():return {'pane_id':s.get('pane','w1:p1'),'workspace_id':'w1','tab_id':'w1:t1','terminal_id':'term'+str(s['creates']),'cwd':s.get('cwd','')}
def agent():return dict(pane(),agent='claude',name='hp-demo-coordinator',agent_status=s.get('status','working' if s['accepted'] else 'idle'),interactive_ready=True)
if a==['--version']:print('herdr 0.9.1');sys.exit(0)
if a==['remote-api-bridge']:
 r=json.loads(sys.stdin.readline());m=r['method'];p=r.get('params') or {};res=None
 if m=='pane.list':res={'panes':[pane()] if s['live'] else []}
 elif m=='agent.list':res={'type':'agent_list','agents':[agent()] if s['live'] and s['agent'] else []}
 elif m=='agent.explain':res={'explain':{'agent':'claude','state':s.get('status','working' if s['accepted'] else 'idle'),'manifest_source':s.get('manifest_source','bundled'),'manifest_version':'fixture-1','matched_rule':{'id':'prompt','state':'idle'},'visible_idle':s.get('status','working' if s['accepted'] else 'idle') in ['idle','done'],'visible_working':s.get('status','working' if s['accepted'] else 'idle')=='working','visible_blocker':False,'screen_detection_skipped':False,'skip_state_update':False,'local_override_shadowing_remote':s.get('shadow',False),'fallback_reason':None,'warning':None}}
 elif m=='notification.show':res={'type':'notification_show','shown':not s.get('toast_busy',False),'reason':'busy' if s.get('toast_busy',False) else 'shown'}
 elif m=='agent.prompt':
  s['prompts'].append(p['text'])
  s.pop('status',None)
  s['accepted']=len(s['prompts'])>s.get('swallow',0)
  res={'type':'agent_prompted','agent':agent()}
 else:res={'type':'ok'}
 reply={'id':r['id'],'result':res}
elif a[:2]==['workspace','create']:
 s['creates']+=1;s.update(live=True,agent=False,accepted=False,cwd=a[a.index('--cwd')+1],pane='w1:p'+str(s['creates']))
 reply={'result':{'type':'workspace_created','workspace':{'workspace_id':'w1'},'pane':pane(),'root_pane':pane(),'tab':{'tab_id':'w1:t1'}}}
elif a[:2]==['agent','start']:
 s['starts']+=1;s['agent']=True;s['args']=a;reply={'result':{'agent':agent()}}
elif a==['pane','list']:reply={'result':{'panes':[pane()] if s['live'] else []}}
elif a==['agent','list']:
 if s.get('fail_inventory'):sys.exit(1)
 reply={'result':{'agents':[agent()] if s['agent'] else []}}
else:reply={'result':{'type':'ok'}}
json.dump(s,open(path,'w'))
if a==['remote-api-bridge'] and m=='agent.prompt' and os.path.exists(root+'/lose-prompt-reply'):
 os.unlink(root+'/lose-prompt-reply');sys.exit(1)
print(json.dumps(reply))
