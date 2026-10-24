"""Atomic new-folder exports of native geometry and paired server entrances."""
import json
from pathlib import Path
import shutil
import tempfile
from preview_assets import Assets
from world_project import OverlayDat,digest
from world_dungeon import DungeonBuilder
from world_dat_export import export_archive
from world_entrance import pair_rows,render_routes,validate_pair
from editor_document import rows_from_text

def entrance_objects(pairs,version=3):
    return [row.split('\t') for pair in pairs for row in pair_rows(pair,version)[0]]

def export_bundle(project,destination,document):
    destination=Path(destination).absolute()
    if destination.exists():raise ValueError('Choose a new export folder; existing folders are never replaced')
    rows=rows_from_text(document)
    version=3 if document.splitlines()[0].endswith('3') else 2
    extra=entrance_objects(project.entrances,version)
    existing={row[0]:row for row in rows}
    additions=[]
    for row in extra:
        if row[0] in existing:
            if existing[row[0]]!=row:raise ValueError('An entrance object ID is already used by a different object placement')
        else:additions.append(row)
    extra=additions
    content=f'pyrealium-world-v{version}\n'+'\n'.join('\t'.join(row) for row in rows+extra)+'\n'
    rows_from_text(content)
    routes=render_routes(project.entrances)
    assets=Assets(project.source)
    try:
        assets.cells=OverlayDat(assets.cells,project)
        assets.portal=OverlayDat(assets.portal,project,'portal.dat')
        builder=DungeonBuilder(assets.cells,assets.portal)
        blocks=sorted({did>>16 for did in project.patches if 0x100<=did&65535<=0xfffb})
        errors=[error for block in blocks for error in builder.validate(block)]
        if errors:raise ValueError('Finish dungeon connections before export:\n'+'\n'.join(errors[:20]))
        index=assets.cells.index()
        for pair in project.entrances:
            pair=validate_pair(pair)
            for key in ('entrance','return'):
                for field in ('source','destination'):
                    cell=pair[key][field]['cell'];did=cell|65535 if cell&65535<256 else cell
                    if did not in index:raise ValueError(f'Entrance references missing area {cell:08x}')
    finally:assets.close()
    destination.parent.mkdir(parents=True,exist_ok=True)
    stage=Path(tempfile.mkdtemp(prefix=destination.name+'.stage-',dir=destination.parent))
    try:
        reports={}
        for archive,patches,originals in (('cell_1.dat',project.patches,project.originals),('portal.dat',project.portal_patches,project.portal_originals)):
            if patches:
                reports[archive]=export_archive(project.source/archive,stage/archive,patches,{did:digest(originals.get(did)) for did in patches})
        (stage/'world.tsv').write_text(content,encoding='utf-8')
        (stage/'world.routes.tsv').write_text(routes,encoding='utf-8')
        project.save(stage/'world.world.json')
        (stage/'README.txt').write_text('WorldBuilder export\n\nChanged native archives are included. Use a separate copy of the original game-data folder and place these archives in it. Unchanged DATs are still required from the same source installation.\n\nUse world.tsv as the server content file; world.routes.tsv must remain beside it. The updated server loads paired portal routes with this content file when portal travel is enabled (including serve-training).\n\nNo installed files were replaced and no active client/server was restarted. Native client traversal remains a separate acceptance check.\n',encoding='utf-8')
        (stage/'export.json').write_text(json.dumps({'format':'pyrealium-world-export-v1','source':str(project.source),'archives':list(reports),'entrance_pairs':len(project.entrances)},indent=2),encoding='utf-8')
        if destination.exists():raise ValueError('Export folder appeared during export; nothing replaced')
        stage.rename(destination)
        return {'destination':str(destination),'archives':list(reports),'entrance_pairs':len(project.entrances)}
    finally:
        if stage.exists() and stage.parent.resolve()==destination.parent.resolve() and stage.name.startswith(destination.name+".stage-"):
            shutil.rmtree(stage)
