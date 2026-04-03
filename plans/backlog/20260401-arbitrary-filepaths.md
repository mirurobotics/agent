
## Customer Conversation

Hey! I am working on ensuring that the agent is ready for integration with a partner's `cell_configuration` config in the pilot. From our conversations, there are two key integration points that we've discussed:

Delivery to the proper file system location via the agent
Integration with the partner's cloud services

In the thread, I've outlined our understanding of the partner's current workflow using the `cell_configuration` config.

Afterward, I've attached some clarifying questions for both integration points.
26 repliesEngineer A [11:52 AM]
Here's our understanding of how the partner currently uses the `cell_configuration` config:


The config is defined as a YAML file in S3 containing various parameters, including ROS launch arguments
To update the config, partner team members pull down the YAML file, make some edits, and upload it back to S3
To deliver the config to a device, on every boot, a systemd service invokes a script to fetch the config from S3, format the launch args as a `.conf` file, and writes to `/var/local/app/configuration/cell.conf`
The systemd service responsible for booting the application supplies ROS launch args from the `.conf` file (via `EnvironmentFile=` setting in systemd)
[11:52 AM]Agent integration questions:


Does the cell.conf file contain only the ROS2 launch args from the YAML file? Or does it contain other parameters besides the ROS2 launch args?
cell.conf is formatted in a simple key-value format (<key> = <value>), yes?
Is the YAML file actually placed anywhere on the filesystem? Or is it only used to produce the .conf file?
(edited)
[11:52 AM]Cloud services integration questions:


Is the primary endpoint just to read the current deployment for a given device from the service?
Is there any other API functionality you will need?
(edited)
Engineer B [11:54 AM]
Does the cell.conf file contain only the ROS2 launch args from the YAML file? Or does it contain other parameters besides the ROS2 launch args?

Yes, that's correct. Additionally, there is a mechanism for local overrides in a separate YAML file.
cell.conf is formatted in a simple key-value format (<key> = <value>), yes?

Yes, but don't worry about integrating this into the agent. I just need access to the YAML.

Is the YAML file actually placed anywhere on the filesystem? Or is it only used to produce the .conf file?

Yes, it is currently placed in `/var/local/app/configuration/cell_configuration.yaml`
[11:55 AM]Is the primary endpoint just to read the current deployment for a given device from the service?

Yes this will be sufficient for pilot

Is there any other API functionality you will need?

As we look at bringing in additional information, ability to update the configuration from an upstream system we are migrating from would be handy.
Engineer A [11:59 AM]
Perfect, thanks for the quick answers! Updating configurations is supported in our API, so we should be good there
Engineer B [11:59 AM]
Sweeeeet
[12:01 PM]You caught me at a good time! I just drank a ton of coffee so keyboard go brr (edited) 
Engineer A [12:01 PM]
We are working on allowing you to specify arbitrary file paths on your file system that the agent can write to so that it can write directly to `/var/local/app/configuration/cell_configuration.yaml`. Just to double-check, this is what you want/expect (that the agent writes directly to `/var/local/app/configuration/cell_configuration.yaml`)?
[12:02 PM]almost time for another cup :thinking_face: :rolling_on_the_floor_laughing:
Engineer B [12:02 PM]
Yes arbitrary paths would be very useful.  We can probably make it work without but would need more "glue" scripts
[12:02 PM]Goal is minimal code change for consuming apps
Engineer A [12:03 PM]
100%
Engineer B [12:03 PM]
By the way, we can make any permission changes as necessary on the filesystem with an Ansible playbook - that is no issue
Engineer A [12:03 PM]
To do this you would need to add a systemd override to the base agent systemd service definition (we will provide documentation of everything of course). Does this work?
Engineer B [12:03 PM]
Used to doing the same thing with docker group
Engineer A [12:03 PM]
you read my mind haha
Engineer B [12:04 PM]
you would need to add a systemd override to the base miru systemd service

Is this where all configs would land? Or is this a group selection?
[12:05 PM]I'm imagining a future state with a templating system that can land arbitrary configuration values in named directories
Engineer A [12:06 PM]
This is just a small `systemd.service` override file which specifies the file system paths that are read/write accessible to the agent. Actual configurations would be written to anywhere on the file system you specify (as long as that file system path is allowed in the systemd definition, of course)
[12:07 PM]it would like like this basically:
# /etc/systemd/system/agent.service.d/write-paths.conf
  [Service]                                                                                               
  ReadWritePaths=/var/local/app/configuration
Engineer B [12:07 PM]
Is that systemd configuration a custom enforcement aside from filesystem group access?
Engineer A [12:08 PM]
yes it's enforced at the kernel level so it's a bit stronger than the group access
Engineer B [12:08 PM]
OK this model sounds workable - I'm sure I'll understand once I get my hands on it
[12:08 PM]Thanks for the updates!
Engineer A [12:09 PM]
of course, thanks for the quick response!

## Feature

Currently Miru forces all config instance file paths to live under the `/srv/miru/config_instances` directory. However, many customers want the config to be delivered to an arbitrary location on their filesystem, such `/var/local/` or `/home/` or somewhere else. 

## Considerations

## Systemd Permissions

### Problem

The Miru `systemd` service file prohibits writing to file system paths which are not prefixed with `/miru/srv/`. 

### Solution

Customers will need to add a `systemd` override file like the following to allow for configurations to be written to the file paths of their choosing:

```
# /etc/systemd/system/miru.service.d/write-paths.conf                                                   
  [Service]                                                                                               
  ReadWritePaths=/var/local/forge/configuration
```

For now, we'll need to provide good documentation on this integration step, no other programming needs to be done.

## File System Permissions

### Problem

The `systemd` override only lifts the sandbox restriction — it does not grant Unix filesystem permissions. The Miru agent runs as `User=miru` / `Group=miru`, so it still needs standard Unix write access to the target path. Without this, writes will fail with a permission denied error even if `ReadWritePaths` allows the path.

Additionally, the agent uses atomic writes (write to a temp file, then rename into place), so the `miru` user needs write permission on the **parent directory**, not just the target file itself. All ancestor directories must also be traversable (`x` bit).

### Solution

Customers need to grant the `miru` user write access to the target directory using a POSIX ACL. This is the least invasive approach — it does not change existing ownership, group, or permission bits on the directory.

```bash
sudo setfacl -m u:miru:rwx /var/local/app/configuration
```

This is a one-time setup step (e.g. in an Ansible playbook) alongside the `systemd` override. Both are required:

| Step | What it does |
|------|-------------|
| `systemd` override (`ReadWritePaths=...`) | Lifts the kernel-level sandbox restriction |
| `setfacl -m u:miru:rwx <dir>` | Grants the `miru` user write access without changing ownership or group |

ACLs are supported out of the box on Ubuntu 24.04 (ext4/xfs have ACL support enabled by default, and the `acl` package is pre-installed). No extra dependencies or filesystem changes are needed.

We will need to document both steps together for customers.

## Atomic Deployments


b. The Miru agent currently makes deployments by collecting all of the instances into a directory that can be atomically replaced with the `/miru/srv/config_instances` directory. This won't work for arbitrary file paths since their isn't a way to atomically write to multiple places onthe file system at the same time.