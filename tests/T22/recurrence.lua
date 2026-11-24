local callbacks, timers, monitors, workspaces = {}, {}, {}, {}
local calls, moves = {}, {}
hl={}
hl.get_monitors=function() return monitors end
hl.get_workspaces=function() return workspaces end
hl.on=function(event, fn) callbacks[event]=fn end
hl.timer=function(fn, options) local t={fn=fn,options=options,set_enabled=function(self,v) self.enabled=v end}; timers[#timers+1]=t; return t end
hl.monitor=function(settings) calls[#calls+1]=settings end
hl.dsp={workspace={move=function(move) return move end}}
hl.dispatch=function(move) moves[#moves+1]={id=move.workspace.id,target=move.monitor.name}; move.workspace.monitor=move.monitor end
local function monitor(name,serial,x) return {name=name,make='Synthetic',model='Topology contract',serial=serial,width=1920,height=1080,x=x,y=0,scale=1,transform=0,dpms_status=true,is_mirror=false} end
local a,b=monitor('FIXTURE-A','A',0),monitor('FIXTURE-B','B',1920)
local target={id=1,config_name='1',monitor=a}; local decoy={id=99,config_name='99',monitor=a}; local special={id=-1,config_name='special:keep',special=true,monitor=b}
workspaces={target,decoy,special}
dofile(arg[1])
assert(#timers==1 and timers[1].options.timeout==2000 and timers[1].options.type=='repeat')
local tick=timers[1].fn
monitors={a,b}; target.monitor=b; tick(); assert(target.monitor==a and decoy.monitor==a and special.monitor==b)
-- Geometry never settles: exactly three attempts, then no more.
a.x=777; calls={}; for i=1,12 do tick() end; assert(#calls==3); assert(calls[1].output=='FIXTURE-A' and calls[1].position=='0x0')
-- Leaving and returning resets this visit's retry budget.
monitors={a}; a.x=0; target.monitor=b; tick(); assert(target.monitor==a)
monitors={b,a}; a.x=777; calls={}; for i=1,12 do tick() end; assert(#calls==3)
-- Convergence resets budget; next drift can be corrected.
a.x=0; tick(); a.x=777; calls={}; tick(); assert(#calls==1)
-- Unknown identity has no saved geometry or fabricated workspace assignments.
local unknown=monitor('UNKNOWN','U',0); monitors={unknown}; target.monitor=a; decoy.monitor=unknown; calls={}; moves={}; tick(); assert(#calls==0 and #moves==1 and moves[1].id==1 and moves[1].target=='UNKNOWN'); assert(#workspaces==3 and special.monitor==b)
-- Unavailable output cannot be a target; empty active set does nothing.
unknown.dpms_status=false; calls={}; moves={}; tick(); assert(#calls==0 and #moves==0 and #workspaces==3)
-- Replacing the policy disables the prior timer.
dofile(arg[1]); assert(#timers==2 and timers[1].enabled==false)
print('PASS: saved-set selection, reorder identity, three-attempt bound, return reset, convergence reset, unknown/empty safety, target/decoy identities, timer replacement')
