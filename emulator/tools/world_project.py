"""Persistent native-record edits, with source verification and read-only overlays."""
import base64
import hashlib
import json
from copy import deepcopy
import os
from pathlib import Path
import tempfile

from ac2_dat import Dat


MAX_BYTES = 64*1024*1024
MAX_RECORDS = 4096


def digest(value):
    return None if value is None else hashlib.sha256(value).hexdigest()


class WorldProject:
    def __init__(self, source):
        self.source = Path(source).resolve()
        self.patches = {}
        self.originals = {}
        self.portal_patches = {}
        self.portal_originals = {}
        self.entrances = []
        self.revision = 0
        self.saved_signature = self.signature()

    def signature(self):
        return (tuple((identity, digest(value)) for identity, value in sorted(self.patches.items())),
                tuple((identity, digest(value)) for identity, value in sorted(self.portal_patches.items())),
                json.dumps(self.entrances,sort_keys=True,allow_nan=False))

    @property
    def dirty(self):
        return self.signature() != self.saved_signature

    def apply(self, changes, archive="cell_1.dat"):
        if archive == "portal.dat":
            child=WorldProject(self.source)
            child.patches,child.originals=dict(self.portal_patches),dict(self.portal_originals)
            before,after=child.apply(changes)
            if sum(map(len,self.patches.values()))+sum(map(len,child.patches.values()))>MAX_BYTES:
                raise ValueError("Project edited data exceeds 64 MiB")
            self.portal_patches,self.portal_originals=child.patches,child.originals
            self.revision+=1
            return {(archive,k):v for k,v in before.items()},{(archive,k):v for k,v in after.items()}
        if archive != "cell_1.dat":raise ValueError("Unsupported project archive")
        """changes maps DID -> (current bytes or None, replacement bytes or None)."""
        if len(set(self.patches) | set(changes)) > MAX_RECORDS:
            raise ValueError("Project exceeds the native-record limit")
        before, after = {}, {}
        for identity, (old, new) in changes.items():
            if not 0 < identity <= 0xffffffff or identity & 65535 in (0xfffc,0xfffd):
                raise ValueError("Invalid or reserved edited record identity")
            if new is not None and (not isinstance(new, bytes) or len(new) > 8*1024*1024):
                raise ValueError("Edited record is outside project bounds")
            current = self.patches.get(identity, self.originals.get(identity, old))
            if current != old:
                raise ValueError(f"Record {identity:08x} changed during the edit")
            before[identity] = old
            after[identity] = new
        originals = dict(self.originals)
        patches = dict(self.patches)
        for identity, (old, new) in changes.items():
            originals.setdefault(identity, old)
            if new == originals[identity]:
                patches.pop(identity, None)
            elif new is None and originals[identity] is not None:
                raise ValueError("Deleting original DAT records is not supported by this project")
            elif new is None:
                patches.pop(identity, None)
            else:
                patches[identity] = new
        if len(patches)+len(self.portal_patches)>MAX_RECORDS:
            raise ValueError("Project exceeds the native-record limit")
        if sum(len(value) for value in patches.values())+sum(len(value) for value in self.portal_patches.values()) > MAX_BYTES:
            raise ValueError("Project edited data exceeds 64 MiB")
        self.originals, self.patches = originals, patches
        self.revision += 1
        return before, after

    def apply_bundle(self, archives):
        """Validate every archive edit before publishing any part of a room."""
        child=WorldProject(self.source)
        for name in ("patches","originals","portal_patches","portal_originals"):
            setattr(child,name,dict(getattr(self,name)))
        before,after={},{}
        for archive,changes in archives.items():
            old,new=child.apply(changes,archive)
            before.update(old);after.update(new)
        if sum(len(v) for v in child.patches.values())+sum(len(v) for v in child.portal_patches.values())>MAX_BYTES:
            raise ValueError("Project edited data exceeds 64 MiB")
        if len(child.patches)+len(child.portal_patches)>MAX_RECORDS:
            raise ValueError("Project exceeds the native-record limit")
        for name in ("patches","originals","portal_patches","portal_originals"):
            setattr(self,name,getattr(child,name))
        self.revision+=1
        return before,after

    def set_entrances(self, entries):
        if not isinstance(entries,list) or len(entries)>256:
            raise ValueError("Entrance count exceeds project bounds")
        from world_entrance import validate_pair,render_routes
        entries=[validate_pair(entry) for entry in entries]
        render_routes(entries)
        encoded=json.dumps(entries,allow_nan=False)
        if len(encoded)>1024*1024:raise ValueError("Entrance data exceeds project bounds")
        before={("entrances",0):deepcopy(self.entrances)}
        self.entrances=deepcopy(entries);self.revision+=1
        return before,{("entrances",0):deepcopy(entries)}

    def restore(self, values):
        portal={key[1]:value for key,value in values.items() if isinstance(key,tuple) and key[0]=="portal.dat"}
        if ("entrances",0) in values:self.entrances=deepcopy(values[("entrances",0)])
        for identity,value in portal.items():
            if value is None or value==self.portal_originals.get(identity):self.portal_patches.pop(identity,None)
            else:self.portal_patches[identity]=value
        values={key:value for key,value in values.items() if not isinstance(key,tuple)}
        patches = dict(self.patches)
        for identity, value in values.items():
            if value is None or value == self.originals.get(identity):
                patches.pop(identity, None)
            else:
                patches[identity] = value
        self.patches = patches
        self.revision += 1

    def save(self, path):
        path = Path(path)
        def records(patches, originals):
            return [dict(id=f"{identity:08x}",original_sha256=digest(originals.get(identity)),data=base64.b64encode(value).decode("ascii")) for identity,value in sorted(patches.items())]
        data = dict(format="pyrealium-world-project-v2",source=str(self.source),
                    archives={"cell_1.dat":records(self.patches,self.originals),"portal.dat":records(self.portal_patches,self.portal_originals)},
                    entrances=self.entrances)
        raw = json.dumps(data,indent=2).encode("utf-8")
        if len(raw) > MAX_BYTES*2:
            raise ValueError("Project manifest exceeds its saved-file limit")
        path.parent.mkdir(parents=True,exist_ok=True)
        descriptor, name = tempfile.mkstemp(prefix=path.name+".",suffix=".tmp",dir=path.parent)
        try:
            with os.fdopen(descriptor,"wb") as file:
                file.write(raw)
                file.flush()
                os.fsync(file.fileno())
            Path(name).replace(path)
        finally:
            Path(name).unlink(missing_ok=True)
        self.saved_signature = self.signature()

    @classmethod
    def load(cls, path, source=None):
        with Path(path).open("rb") as file:
            raw = file.read(MAX_BYTES*2+1)
        if len(raw) > MAX_BYTES*2:
            raise ValueError("Project file exceeds its saved-file limit")
        data = json.loads(raw)
        version=data.get("format")
        if version=="pyrealium-world-project-v1" and data.get("archive")=="cell_1.dat":
            archives={"cell_1.dat":data.get("records",[])}
        elif version=="pyrealium-world-project-v2":
            archives=data.get("archives",{})
            if not isinstance(archives,dict) or set(archives)-{"cell_1.dat","portal.dat"}:raise ValueError("Unsupported project archive")
        else:raise ValueError("Unsupported world project format")
        project = cls(source or data["source"])
        count=0
        for archive,records in archives.items():
            if not isinstance(records,list):raise ValueError("Invalid project record list")
            count+=len(records)
            if count>MAX_RECORDS:raise ValueError("Project exceeds the record limit")
            if not records:continue
            with Dat(project.source/archive) as cells:
                index=cells.index();seen=set()
                for row in records:
                    identity=int(row["id"],16)
                    if identity in seen:raise ValueError("Duplicate project record")
                    seen.add(identity)
                    original=cells.read(identity) if identity in index else None
                    if digest(original)!=row["original_sha256"]:raise ValueError(f"Project source differs for record {identity:08x}")
                    value=base64.b64decode(row["data"],validate=True)
                    project.apply({identity:(original,value)},archive)
        project.set_entrances(data.get("entrances",[]))
        project.saved_signature = project.signature()
        return project


class OverlayDat:
    """A Dat-compatible read view. The base archive is never mutated."""
    def __init__(self, base, project, archive="cell_1.dat"):
        self.base, self.project = base, project
        self.archive=archive
        self.path = base.path

    @property
    def patches(self):
        return self.project.portal_patches if self.archive=="portal.dat" else self.project.patches

    def read(self, identity):
        value = self.patches.get(identity)
        return value if value is not None else self.base.read(identity)

    def index(self):
        result = dict(self.base.index())
        result.update({identity:(0,len(value),0) for identity,value in self.patches.items()})
        return result

    def close(self):
        self.base.close()
