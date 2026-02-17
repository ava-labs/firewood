#!/usr/bin/env python3
"""
Verify that all pinned GitHub Actions in workflow files use valid commit hashes.
This script checks that each pinned action's commit hash is fetchable from GitHub
and optionally verifies that tags match the pinned commits.

Set GITHUB_TOKEN environment variable to avoid rate limiting.
"""

import os
import re
import sys
import urllib.request
import urllib.error
from pathlib import Path
from typing import List, Tuple

def find_workflow_files(workflows_dir: Path) -> List[Path]:
    """Find all YAML workflow files."""
    return list(workflows_dir.glob("*.yaml")) + list(workflows_dir.glob("*.yml"))

def extract_pinned_actions(file_path: Path) -> List[Tuple[str, str, str, int]]:
    """
    Extract all pinned actions (those using commit hashes) from a workflow file.
    Returns list of (owner/repo, commit_hash, tag_name, line_number) tuples.
    tag_name will be empty string if no tag comment found.
    """
    actions = []
    # Match: uses: owner/repo@<40-char-hex-hash> with optional comment # @tag or # @v1 -> v1.2.3
    pattern = re.compile(r'uses:\s+([^@\s]+)@([0-9a-f]{40})(?:\s+#\s+@?([^\s]+))?')
    
    with open(file_path, 'r') as f:
        for line_num, line in enumerate(f, 1):
            match = pattern.search(line)
            if match:
                repo = match.group(1)
                commit_hash = match.group(2)
                tag_name = match.group(3) if match.group(3) else ""
                # Strip any " -> version" suffix to get the base tag
                if tag_name and " -> " in tag_name:
                    tag_name = tag_name.split(" -> ")[0]
                actions.append((repo, commit_hash, tag_name, line_num))
    
    return actions

def verify_commit_exists(repo: str, commit_hash: str) -> Tuple[bool, str]:
    """
    Verify that a commit exists in a GitHub repository.
    Returns (success, message) tuple.
    """
    # Use GitHub API to check if commit exists
    url = f"https://api.github.com/repos/{repo}/commits/{commit_hash}"
    
    try:
        req = urllib.request.Request(url)
        req.add_header('Accept', 'application/vnd.github.v3+json')
        
        # Add GitHub token if available to avoid rate limiting
        github_token = os.environ.get('GITHUB_TOKEN')
        if github_token:
            req.add_header('Authorization', f'token {github_token}')
        
        with urllib.request.urlopen(req, timeout=10) as response:
            if response.status == 200:
                return True, "OK"
            else:
                return False, f"HTTP {response.status}"
    except urllib.error.HTTPError as e:
        if e.code == 404:
            return False, "Commit not found (404)"
        elif e.code == 403:
            return False, "Rate limited (403) - consider setting GITHUB_TOKEN"
        else:
            return False, f"HTTP error {e.code}"
    except urllib.error.URLError as e:
        return False, f"Network error: {e.reason}"
    except Exception as e:
        return False, f"Error: {str(e)}"

def verify_tag_points_to_commit(repo: str, tag_name: str, expected_commit_hash: str) -> Tuple[bool, str]:
    """
    Verify that a tag points to the expected commit hash.
    Returns (success, message) tuple.
    """
    # Skip verification for non-version tags (moving targets like 'stable', 'main', etc.)
    if tag_name.lower() in ['stable', 'main', 'master', 'latest']:
        return True, f"OK ('{tag_name}' is a moving tag, not verified)"
    
    # Use GitHub API to get the commit that a tag points to
    url = f"https://api.github.com/repos/{repo}/git/ref/tags/{tag_name}"
    
    try:
        req = urllib.request.Request(url)
        req.add_header('Accept', 'application/vnd.github.v3+json')
        
        # Add GitHub token if available to avoid rate limiting
        github_token = os.environ.get('GITHUB_TOKEN')
        if github_token:
            req.add_header('Authorization', f'token {github_token}')
        
        with urllib.request.urlopen(req, timeout=10) as response:
            if response.status == 200:
                import json
                data = json.loads(response.read())
                
                # The ref might point to a tag object or directly to a commit
                if data['object']['type'] == 'commit':
                    actual_commit = data['object']['sha']
                elif data['object']['type'] == 'tag':
                    # Need to dereference the tag object
                    tag_url = data['object']['url']
                    tag_req = urllib.request.Request(tag_url)
                    tag_req.add_header('Accept', 'application/vnd.github.v3+json')
                    if github_token:
                        tag_req.add_header('Authorization', f'token {github_token}')
                    with urllib.request.urlopen(tag_req, timeout=10) as tag_response:
                        tag_data = json.loads(tag_response.read())
                        actual_commit = tag_data['object']['sha']
                else:
                    return False, f"Unknown object type: {data['object']['type']}"
                
                if actual_commit == expected_commit_hash:
                    return True, f"Tag matches commit"
                else:
                    return False, f"Tag mismatch: {tag_name} -> {actual_commit[:8]} (expected {expected_commit_hash[:8]})"
            else:
                return False, f"HTTP {response.status}"
    except urllib.error.HTTPError as e:
        if e.code == 404:
            return False, f"Tag '{tag_name}' not found (404)"
        elif e.code == 403:
            return False, "Rate limited (403) - consider setting GITHUB_TOKEN"
        else:
            return False, f"HTTP error {e.code}"
    except urllib.error.URLError as e:
        return False, f"Network error: {e.reason}"
    except Exception as e:
        return False, f"Error: {str(e)}"

def main():
    """Main function to verify all pinned actions."""
    # Find workflows directory
    repo_root = Path(__file__).parent.parent
    workflows_dir = repo_root / ".github" / "workflows"
    
    if not workflows_dir.exists():
        print(f"Error: Workflows directory not found at {workflows_dir}", file=sys.stderr)
        sys.exit(1)
    
    print("🔍 Scanning workflow files for pinned actions...\n")
    
    # Check for GitHub token
    if not os.environ.get('GITHUB_TOKEN'):
        print("⚠️  GITHUB_TOKEN not set. You may hit rate limits with many requests.")
        print("   Set GITHUB_TOKEN environment variable to avoid this.\n")
    
    # Collect all pinned actions
    all_actions = []
    workflow_files = find_workflow_files(workflows_dir)
    
    for workflow_file in sorted(workflow_files):
        actions = extract_pinned_actions(workflow_file)
        if actions:
            rel_path = workflow_file.relative_to(repo_root)
            for repo, commit_hash, tag_name, line_num in actions:
                all_actions.append((rel_path, repo, commit_hash, tag_name, line_num))
    
    if not all_actions:
        print("✅ No pinned actions found (all using tags)")
        return 0
    
    print(f"Found {len(all_actions)} pinned action(s)\n")
    
    # Verify each action (only once per unique repo+hash+tag)
    failed = []
    succeeded = []
    verified_cache = {}  # Cache of (repo, commit_hash, tag_name) -> (success, message)
    
    for workflow_path, repo, commit_hash, tag_name, line_num in all_actions:
        cache_key = (repo, commit_hash, tag_name)
        
        # Check if we've already verified this exact action
        if cache_key in verified_cache:
            success, message = verified_cache[cache_key]
            tag_display = f"@{tag_name}" if tag_name else ""
            print(f"Checking {repo}@{commit_hash[:8]}{tag_display}... ✓ (cached) {message}")
        else:
            tag_display = f"@{tag_name}" if tag_name else ""
            print(f"Checking {repo}@{commit_hash[:8]}{tag_display}... ", end='', flush=True)
            
            # First verify the commit exists
            success, message = verify_commit_exists(repo, commit_hash)
            
            # If commit exists and we have a tag, verify the tag matches
            if success and tag_name:
                tag_success, tag_message = verify_tag_points_to_commit(repo, tag_name, commit_hash)
                if not tag_success:
                    success = False
                    message = tag_message
                else:
                    message = f"OK (tag {tag_name} verified)"
            
            verified_cache[cache_key] = (success, message)
            
            if success:
                print(f"✅ {message}")
            else:
                print(f"❌ {message}")
        
        if success:
            succeeded.append((workflow_path, repo, commit_hash, tag_name, line_num))
        else:
            failed.append((workflow_path, repo, commit_hash, tag_name, line_num, message))
    
    # Print summary
    print(f"\n{'='*70}")
    print(f"Summary: {len(succeeded)} succeeded, {len(failed)} failed")
    print(f"{'='*70}\n")
    
    if failed:
        print("❌ Failed Actions:\n")
        
        # Separate tag mismatches from other errors
        tag_mismatches = []
        other_errors = []
        
        for workflow_path, repo, commit_hash, tag_name, line_num, error_msg in failed:
            if "Tag mismatch" in error_msg:
                tag_mismatches.append((workflow_path, repo, commit_hash, tag_name, line_num, error_msg))
            else:
                other_errors.append((workflow_path, repo, commit_hash, tag_name, line_num, error_msg))
        
        # Show tag mismatches first with explanation
        if tag_mismatches:
            print("⚠️  Tag Mismatches (pinned commit doesn't match current tag):\n")
            print("   These actions are pinned to a commit that differs from what the tag currently points to.")
            print("   This is SAFE (the pinned commit won't change) but the comment may be outdated.\n")
            for workflow_path, repo, commit_hash, tag_name, line_num, error_msg in tag_mismatches:
                tag_display = f"@{tag_name}" if tag_name else ""
                print(f"  File: {workflow_path}:{line_num}")
                print(f"  Action: {repo}@{commit_hash}{tag_display}")
                print(f"  Issue: {error_msg}")
                print()
        
        # Show other errors
        if other_errors:
            if tag_mismatches:
                print("❌ Other Errors:\n")
            for workflow_path, repo, commit_hash, tag_name, line_num, error_msg in other_errors:
                tag_display = f"@{tag_name}" if tag_name else ""
                print(f"  File: {workflow_path}:{line_num}")
                print(f"  Action: {repo}@{commit_hash}{tag_display}")
                print(f"  Error: {error_msg}")
                print()
        
        # If only tag mismatches, consider it a warning not a failure
        if tag_mismatches and not other_errors:
            print("ℹ️  Note: All failures are tag mismatches, which don't affect security.")
            print("   The pinned commits are still valid and safe to use.")
            return 0
        
        return 1
    else:
        print("✅ All pinned actions verified successfully!")
        return 0

if __name__ == "__main__":
    sys.exit(main())
